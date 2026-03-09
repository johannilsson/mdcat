use anyhow::{bail, Context, Result};
use image::{DynamicImage, GenericImageView};
use std::path::Path;
use crate::render::Config;
use crate::terminal::{detect_image_protocol, ImageProtocol};

/// Render an image (by URL or file path) to an ANSI/terminal-graphics string.
pub fn render_image(url: &str, _alt: &str, base_dir: Option<&Path>, config: &Config) -> Result<String> {
    let img = load_image(url, base_dir)?;
    let (img_px_w, _) = img.dimensions();
    let cell_px = crate::terminal::cell_pixel_width();
    let natural_cols = img_px_w / cell_px.max(1);
    let max_cols = if natural_cols > config.width as u32 {
        Some(config.width)
    } else {
        None
    };
    render_dynamic_image(&img, config, max_cols)
}

/// Render an already-decoded DynamicImage to a terminal graphics string.
///
/// `max_cols`: `Some(n)` forces the image to exactly n columns (scales up or down);
/// `None` lets the terminal display at natural pixel size.
pub fn render_dynamic_image(img: &DynamicImage, config: &Config, max_cols: Option<u16>) -> Result<String> {
    match detect_image_protocol(config.image_protocol.as_deref()) {
        ImageProtocol::ITerm2 => iterm2_encode(img, max_cols),
        ImageProtocol::Kitty => kitty_encode(img, max_cols),
        ImageProtocol::Sixel | ImageProtocol::Blocks => Ok(blocks_encode(img, max_cols.unwrap_or(config.width) as u32)),
    }
}

/// Encode image as an iTerm2 OSC 1337 inline image escape sequence.
fn iterm2_encode(img: &DynamicImage, max_cols: Option<u16>) -> Result<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .context("failed to encode image as PNG for iTerm2")?;
    let b64 = STANDARD.encode(&buf);
    let width_param = match max_cols {
        Some(cols) => format!("width={cols};"),
        None => String::new(),
    };
    Ok(format!(
        "\x1b]1337;File=inline=1;{width_param}preserveAspectRatio=1:{b64}\x07\n"
    ))
}

/// Encode image as a Kitty terminal graphics protocol APC sequence.
///
/// Sends raw RGBA pixel data chunked in 4096-byte base64 blocks.
/// If `max_cols` is Some, `c={cols}` tells Kitty to scale to that many columns.
/// If `max_cols` is None, `c=` is omitted and Kitty displays at natural pixel size.
fn kitty_encode(img: &DynamicImage, max_cols: Option<u16>) -> Result<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let b64 = STANDARD.encode(rgba.as_raw());

    let cols_param = match max_cols {
        Some(cols) => format!(",c={cols}"),
        None => String::new(),
    };

    let mut out = String::new();
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    let total = chunks.len();

    for (i, chunk) in chunks.iter().enumerate() {
        let chunk_str = std::str::from_utf8(chunk).unwrap();
        let more = if i + 1 < total { 1 } else { 0 };
        if i == 0 {
            out.push_str(&format!(
                "\x1b_Ga=T,f=32,s={w},v={h}{cols_param},q=2,m={more};{chunk_str}\x1b\\"
            ));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk_str}\x1b\\"));
        }
    }
    out.push('\n');
    Ok(out)
}

/// Encode image as Unicode half-block characters with truecolor ANSI.
///
/// Each terminal cell represents a 1×2 pixel block: the upper half-block (▀)
/// uses the foreground color for the top pixel and background for the bottom.
/// Scales the image to fit within `max_cols` character columns.
fn blocks_encode(img: &DynamicImage, max_cols: u32) -> String {
    use image::GenericImageView;

    let (orig_w, orig_h) = img.dimensions();
    if orig_w == 0 || orig_h == 0 {
        return String::new();
    }

    // Scale to terminal width; height halved because each cell covers 2 pixel rows
    let target_w = max_cols.min(orig_w);
    let target_h = ((orig_h as f64 * target_w as f64 / orig_w as f64) as u32).max(2);
    let target_h = (target_h + 1) & !1; // round up to even

    let scaled = img.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3);
    let rgba = scaled.to_rgba8();
    let (w, h) = rgba.dimensions();

    let mut out = String::new();
    let mut row = 0u32;
    while row + 1 < h {
        for col in 0..w {
            let top = rgba.get_pixel(col, row);
            let bot = rgba.get_pixel(col, row + 1);
            out.push_str(&format!(
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                top[0], top[1], top[2],
                bot[0], bot[1], bot[2],
            ));
        }
        out.push_str("\x1b[0m\n");
        row += 2;
    }
    out.push('\n');
    out
}

/// Load an image from a file path or URL.
fn load_image(url: &str, base_dir: Option<&Path>) -> Result<DynamicImage> {
    if url.starts_with("http://") || url.starts_with("https://") {
        load_remote_image(url)
    } else if url.starts_with("data:") {
        bail!("data: URI images not yet supported")
    } else {
        load_local_image(url, base_dir)
    }
}

fn load_local_image(url: &str, base_dir: Option<&Path>) -> Result<DynamicImage> {
    let path = url.strip_prefix("file://").unwrap_or(url);
    let path = Path::new(path);

    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base) = base_dir {
        base.join(path)
    } else {
        path.to_path_buf()
    };

    if resolved.extension().map(|e| e.eq_ignore_ascii_case("svg")).unwrap_or(false) {
        let data = std::fs::read(&resolved)
            .with_context(|| format!("failed to read SVG '{}'", resolved.display()))?;
        rasterize_svg(&data)
    } else {
        image::open(&resolved)
            .with_context(|| format!("failed to open image '{}'", resolved.display()))
    }
}

pub fn rasterize_svg(svg_data: &[u8]) -> Result<DynamicImage> {
    use resvg::usvg::{Options, Tree};
    use tiny_skia::{Pixmap, Transform};

    let mut options = Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = Tree::from_data(svg_data, &options)
        .context("failed to parse SVG")?;

    let size = tree.size();
    let width = size.width() as u32;
    let height = size.height() as u32;

    let mut pixmap = Pixmap::new(width, height)
        .context("failed to create pixmap")?;

    resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());

    let rgba_data = pixmap.data().to_vec();
    let img = image::RgbaImage::from_raw(width, height, rgba_data)
        .context("failed to convert pixmap to image")?;

    Ok(DynamicImage::ImageRgba8(img))
}

#[cfg(feature = "network-images")]
fn load_remote_image(url: &str) -> Result<DynamicImage> {
    let bytes = ureq::get(url)
        .call()
        .with_context(|| format!("failed to fetch image '{url}'"))?
        .into_reader();

    let mut buf = Vec::new();
    use std::io::Read;
    bytes.take(50 * 1024 * 1024).read_to_end(&mut buf)?; // 50MB limit

    image::load_from_memory(&buf)
        .context("failed to decode remote image")
}

#[cfg(not(feature = "network-images"))]
fn load_remote_image(url: &str) -> Result<DynamicImage> {
    bail!("network image support not compiled in (build with --features network-images): {url}")
}
