use anyhow::{bail, Context, Result};
use std::process::Command;
use crate::render::Config;
use crate::render::images::{rasterize_svg, render_dynamic_image};

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

    // Create temp file for SVG output
    let output_file = tempfile::Builder::new()
        .prefix("mdcat-mermaid-")
        .suffix(".svg")
        .tempfile()
        .context("failed to create temp file for Mermaid output")?;

    // Run mmdc
    let status = Command::new(mmdc)
        .args([
            "-i", input_file.path().to_str().unwrap(),
            "-o", output_file.path().to_str().unwrap(),
            "-b", "transparent",
        ])
        .output()
        .with_context(|| format!("failed to run '{mmdc}'"))?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        bail!("mmdc failed: {stderr}");
    }

    // Read SVG output
    let svg_data = std::fs::read(output_file.path())
        .context("failed to read Mermaid SVG output")?;

    if svg_data.is_empty() {
        bail!("mmdc produced empty SVG output");
    }

    // Rasterize SVG and render via viuer
    let img = rasterize_svg(&svg_data)
        .context("failed to rasterize Mermaid SVG")?;

    render_dynamic_image(&img, config)
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
