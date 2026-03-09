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
        ])
        .output()
        .with_context(|| format!("failed to run '{mmdc}'"))?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        bail!("mmdc failed: {stderr}");
    }

    // Load PNG and render at natural size (no forced column width)
    let img = image::open(output_file.path())
        .context("failed to load Mermaid PNG output")?;

    render_dynamic_image(&img, config, None)
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
