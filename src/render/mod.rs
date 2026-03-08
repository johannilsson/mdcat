use anyhow::Result;
use crate::input::Source;

mod markdown;
mod code;
mod images;
mod mermaid;
mod links;

pub use markdown::render_document;

/// Rendering configuration passed through the pipeline.
#[derive(Clone)]
pub struct Config {
    pub width: u16,
    pub images: bool,
    pub mermaid: bool,
    pub mermaid_binary: String,
    pub theme: String,
    pub image_protocol: Option<String>,
}

/// Render all sources to a single ANSI string.
/// Multiple files are separated by a dim header line.
pub fn render_all(sources: &[Source], config: &Config) -> Result<String> {
    let mut output = String::new();

    for (i, source) in sources.iter().enumerate() {
        if i > 0 {
            // Separator between files
            let width = config.width as usize;
            let header = format!(" {} ", source.name);
            let dashes = "─".repeat((width.saturating_sub(header.len() + 2)).max(4));
            output.push_str(&format!("\x1b[2m{dashes}{header}{dashes}\x1b[0m\n\n"));
        }

        let rendered = render_document(&source.content, source.base_dir.as_deref(), config)?;
        output.push_str(&rendered);
    }

    Ok(output)
}
