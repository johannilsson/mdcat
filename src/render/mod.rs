use anyhow::Result;
use std::cell::RefCell;
use std::rc::Rc;

use crate::input::Source;

mod markdown;
mod code;
mod images;
mod mermaid;
mod links;

pub use markdown::render_document;

/// In-progress store that accumulates images captured during Kitty-pager rendering.
///
/// When `Config::kitty_store` is `Some`, `render_dynamic_image` stores RGBA data
/// here instead of emitting Kitty escape sequences, and returns a sentinel string
/// so that `build_kitty_document` can reconstruct the image positions.
pub struct KittyImageStore {
    pub images: Vec<kitty_pager::KittyImage>,
    next_id: u32,
}

impl KittyImageStore {
    pub fn new() -> Self {
        Self { images: Vec::new(), next_id: 1 }
    }

    /// Add an image and return its assigned ID.
    pub fn push(
        &mut self,
        rgba_data: Vec<u8>,
        pixel_width: u32,
        pixel_height: u32,
        display_cols: Option<u16>,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.images.push(kitty_pager::KittyImage {
            id,
            rgba_data,
            pixel_width,
            pixel_height,
            display_cols,
        });
        id
    }
}

/// Rendering configuration passed through the pipeline.
#[derive(Clone)]
pub struct Config {
    pub width: u16,
    pub images: bool,
    pub mermaid: bool,
    pub mermaid_binary: String,
    pub theme: String,
    pub image_protocol: Option<String>,
    /// When `Some`, images are captured into the store instead of being encoded
    /// as escape sequences.  Used by the Kitty pager path.
    pub kitty_store: Option<Rc<RefCell<KittyImageStore>>>,
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

/// Build a `KittyDocument` from a rendered ANSI string and the image store.
///
/// The `rendered` string contains sentinel tokens of the form
/// `\x00KITTY:{id}:{cols}:{pw}:{ph}\x00\n` at the position of each image.
/// This function splits the string on those sentinels and builds a
/// `Vec<DocItem>` interleaving text and image items.
pub fn build_kitty_document(
    rendered: &str,
    store: &KittyImageStore,
) -> kitty_pager::KittyDocument {
    use kitty_pager::{DocItem, KittyDocument};

    let mut items = Vec::new();
    let mut remaining = rendered;

    while let Some(start) = remaining.find("\x00KITTY:") {
        // Text before the sentinel.
        let text_part = &remaining[..start];
        if !text_part.is_empty() {
            items.push(DocItem::Text(text_part.to_string()));
        }

        // Find the closing \x00 after "KITTY:..."
        let after_marker = &remaining[start + 1..]; // skip the opening \x00
        if let Some(end) = after_marker.find('\x00') {
            let sentinel = &after_marker[..end]; // "KITTY:{id}:{cols}:{pw}:{ph}"
            remaining = &after_marker[end + 1..]; // skip closing \x00
            // Skip the trailing newline emitted by the sentinel format.
            if remaining.starts_with('\n') {
                remaining = &remaining[1..];
            }

            if let Some(img) = parse_sentinel(sentinel, store) {
                items.push(DocItem::Image(img));
            }
        } else {
            break;
        }
    }

    if !remaining.is_empty() {
        items.push(DocItem::Text(remaining.to_string()));
    }

    KittyDocument { items }
}

fn parse_sentinel(sentinel: &str, store: &KittyImageStore) -> Option<kitty_pager::KittyImage> {
    // sentinel = "KITTY:{id}:{cols}:{pw}:{ph}"
    let rest = sentinel.strip_prefix("KITTY:")?;
    let mut parts = rest.splitn(4, ':');
    let id: u32 = parts.next()?.parse().ok()?;
    let cols: u16 = parts.next()?.parse().ok()?;

    let img = store.images.iter().find(|i| i.id == id)?;
    Some(kitty_pager::KittyImage {
        id: img.id,
        rgba_data: img.rgba_data.clone(),
        pixel_width: img.pixel_width,
        pixel_height: img.pixel_height,
        display_cols: if cols == 0 { None } else { Some(cols) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store_with_image() -> KittyImageStore {
        let mut store = KittyImageStore::new();
        store.push(vec![0u8; 4 * 10 * 10], 10, 10, Some(20));
        store
    }

    #[test]
    fn build_kitty_document_text_only() {
        let store = KittyImageStore::new();
        let doc = build_kitty_document("hello\nworld", &store);
        assert_eq!(doc.items.len(), 1);
        match &doc.items[0] {
            kitty_pager::DocItem::Text(s) => assert_eq!(s, "hello\nworld"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn build_kitty_document_parses_sentinel() {
        let store = make_store_with_image();
        // The image was assigned id=1, cols=20.
        let rendered = "before\n\x00KITTY:1:20:10:10\x00\nafter";
        let doc = build_kitty_document(rendered, &store);
        assert_eq!(doc.items.len(), 3);
        match &doc.items[0] {
            kitty_pager::DocItem::Text(s) => assert_eq!(s, "before\n"),
            _ => panic!("expected Text"),
        }
        match &doc.items[1] {
            kitty_pager::DocItem::Image(img) => {
                assert_eq!(img.id, 1);
                assert_eq!(img.display_cols, Some(20));
            }
            _ => panic!("expected Image"),
        }
        match &doc.items[2] {
            kitty_pager::DocItem::Text(s) => assert_eq!(s, "after"),
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn build_kitty_document_zero_cols_means_none() {
        let mut store = KittyImageStore::new();
        store.push(vec![0u8; 4], 1, 1, None);
        let rendered = "\x00KITTY:1:0:1:1\x00\n";
        let doc = build_kitty_document(rendered, &store);
        assert_eq!(doc.items.len(), 1);
        match &doc.items[0] {
            kitty_pager::DocItem::Image(img) => assert_eq!(img.display_cols, None),
            _ => panic!("expected Image"),
        }
    }
}
