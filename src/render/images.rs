use anyhow::{bail, Context, Result};
use image::DynamicImage;
use std::path::Path;
use crate::render::Config;
use crate::terminal::detect_image_protocol;

/// Render an image (by URL or file path) to an ANSI/terminal-graphics string.
/// Returns an empty string if the image cannot be loaded or rendered.
pub fn render_image(url: &str, _alt: &str, base_dir: Option<&Path>, config: &Config) -> Result<String> {
    let img = load_image(url, base_dir)?;
    render_dynamic_image(&img, config)
}

/// Render an already-decoded DynamicImage to a terminal graphics string.
pub fn render_dynamic_image(img: &DynamicImage, config: &Config) -> Result<String> {
    let protocol = detect_image_protocol(config.image_protocol.as_deref());

    // viuer handles all protocol details; we capture its stdout output
    // by redirecting. viuer prints directly to stdout/stderr, so we use
    // a pipe trick: write to a temp buffer using viuer's print function.
    //
    // Note: viuer prints inline - we return a placeholder string here and
    // render directly. The caller should flush this before continuing output.
    let conf = viuer::Config {
        absolute_offset: false,
        width: Some(config.width as u32),
        use_kitty: protocol == crate::terminal::ImageProtocol::Kitty,
        use_iterm: protocol == crate::terminal::ImageProtocol::ITerm2,
        ..Default::default()
    };

    // viuer prints directly to stdout. We signal to the caller that rendering
    // happened by returning a sentinel; the actual output goes to stdout inline.
    // This matches how viuer is designed to be used.
    viuer::print(img, &conf).context("failed to render image via viuer")?;

    // Return empty string - viuer has already printed to stdout
    Ok(String::new())
}

/// Load an image from a file path or URL.
fn load_image(url: &str, base_dir: Option<&Path>) -> Result<DynamicImage> {
    if url.starts_with("http://") || url.starts_with("https://") {
        load_remote_image(url)
    } else if url.starts_with("data:") {
        load_data_uri(url)
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
        load_svg_file(&resolved)
    } else {
        image::open(&resolved)
            .with_context(|| format!("failed to open image '{}'", resolved.display()))
    }
}

fn load_svg_file(path: &Path) -> Result<DynamicImage> {
    let data = std::fs::read(path)
        .with_context(|| format!("failed to read SVG '{}'", path.display()))?;
    rasterize_svg(&data)
}

pub fn rasterize_svg(svg_data: &[u8]) -> Result<DynamicImage> {
    use resvg::usvg::{Options, Tree};
    use tiny_skia::{Pixmap, Transform};

    let options = Options::default();
    let tree = Tree::from_data(svg_data, &options)
        .context("failed to parse SVG")?;

    let size = tree.size();
    let width = size.width() as u32;
    let height = size.height() as u32;

    let mut pixmap = Pixmap::new(width, height)
        .context("failed to create pixmap")?;

    resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());

    // Convert tiny_skia Pixmap to image::DynamicImage
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

fn load_data_uri(_uri: &str) -> Result<DynamicImage> {
    bail!("data: URI images not yet supported")
}
