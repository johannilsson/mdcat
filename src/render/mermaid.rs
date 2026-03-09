use anyhow::{bail, Context, Result};
use std::process::Command;
use crate::render::Config;
use crate::render::images::render_dynamic_image;

/// Render a Mermaid diagram source string to a terminal image string.
/// Requires `mmdc` (mermaid-cli) to be available on PATH.
pub fn render_mermaid(source: &str, config: &Config) -> Result<String> {
    // Check if mmdc is available
    let mmdc = &config.mermaid_binary;
    if which_mmdc(mmdc).is_err() {
        bail!("'{mmdc}' not found on PATH");
    }

    // Write source to temp file
    let input_file = tempfile::Builder::new()
        .prefix("mdcat-mermaid-")
        .suffix(".mmd")
        .tempfile()
        .context("failed to create temp file for Mermaid input")?;

    std::fs::write(input_file.path(), source)
        .context("failed to write Mermaid source to temp file")?;

    // Create temp file for PNG output — mmdc uses Chromium to render, which
    // handles fonts correctly. This avoids resvg font issues with SVG output.
    let output_file = tempfile::Builder::new()
        .prefix("mdcat-mermaid-")
        .suffix(".png")
        .tempfile()
        .context("failed to create temp file for Mermaid output")?;

    // Run mmdc
    let status = Command::new(mmdc)
        .args([
            "-i", input_file.path().to_str().unwrap(),
            "-o", output_file.path().to_str().unwrap(),
            "-b", "transparent",
            "-t", "dark",
            "-s", "2",
        ])
        .output()
        .with_context(|| format!("failed to run '{mmdc}'"))?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        bail!("mmdc failed: {stderr}");
    }

    // Load PNG and render at natural size, scaling down only if wider than terminal.
    let img = image::open(output_file.path())
        .context("failed to load Mermaid PNG output")?;

    let cell_px = crate::terminal::cell_pixel_width();
    let natural_cols = (img.width() / cell_px).min(u16::MAX as u32) as u16;
    let display_cols = natural_cols.min(config.width);
    render_dynamic_image(&img, config, Some(display_cols))
}

/// Check if the mmdc binary exists and is executable.
fn which_mmdc(binary: &str) -> Result<()> {
    Command::new("which")
        .arg(binary)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| ())
        .context("mmdc not found")
}
