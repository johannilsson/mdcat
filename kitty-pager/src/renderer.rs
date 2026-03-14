use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::HashSet;

use crate::document::{DocItem, KittyDocument};

/// A single terminal row in the laid-out document.
pub(crate) struct LayoutEntry {
    /// Which item in `KittyDocument::items` this entry belongs to.
    pub item_idx: usize,
    /// Content kind.
    pub kind: EntryKind,
}

pub(crate) enum EntryKind {
    Text(String),
    /// One terminal row of an image.  An image spanning N rows produces N
    /// consecutive entries with `row_in_image` going from 0 to N−1.
    ImageRow {
        row_in_image: u32,
        total_rows: u32,
    },
}

/// Build a flat list of terminal rows from a document.
///
/// Text items are split on `\n`; each line becomes one `LayoutEntry`.
/// Image items are expanded into one entry **per terminal row** so that
/// scrolling is row-granular and partial images can be displayed.
pub(crate) fn layout(doc: &KittyDocument, cell_px_height: u32) -> Vec<LayoutEntry> {
    let cell_h = cell_px_height.max(1);
    let mut entries = Vec::new();

    for (idx, item) in doc.items.iter().enumerate() {
        match item {
            DocItem::Text(s) => {
                for line in s.split('\n') {
                    entries.push(LayoutEntry {
                        item_idx: idx,
                        kind: EntryKind::Text(line.to_string()),
                    });
                }
            }
            DocItem::Image(img) => {
                let total_rows = img.pixel_height.div_ceil(cell_h).max(1);
                for row in 0..total_rows {
                    entries.push(LayoutEntry {
                        item_idx: idx,
                        kind: EntryKind::ImageRow {
                            row_in_image: row,
                            total_rows,
                        },
                    });
                }
            }
        }
    }
    entries
}

/// Render a full screen frame to a `String`.
///
/// Images are placed with source-rectangle cropping (`y=`, `r=`) so that
/// partially-visible images render correctly when scrolled.  Pixel data is
/// transmitted once (`a=T`) and re-placed cheaply (`a=p`) on subsequent frames.
pub(crate) fn render_frame(
    doc: &KittyDocument,
    layout: &[LayoutEntry],
    top_entry: usize,
    screen_rows: u16,
    cell_px_height: u32,
    transmitted: &mut HashSet<u32>,
) -> String {
    let mut out = String::new();

    // Delete all image placements (keeps cached pixel data).
    out.push_str("\x1b_Ga=d,d=a,q=2;\x1b\\");
    // Cursor home — no screen clear.
    out.push_str("\x1b[H");

    let max_content_rows = screen_rows.saturating_sub(1) as u32;
    let mut rows_rendered: u32 = 0;
    // Track which images have already been placed in this frame so we skip
    // their continuation rows.
    let mut placed_images: HashSet<usize> = HashSet::new();

    for entry in layout.iter().skip(top_entry) {
        if rows_rendered >= max_content_rows {
            break;
        }

        match &entry.kind {
            EntryKind::Text(line) => {
                out.push_str(line);
                out.push_str("\x1b[K\r\n");
                rows_rendered += 1;
            }
            EntryKind::ImageRow {
                row_in_image,
                total_rows,
            } => {
                if placed_images.contains(&entry.item_idx) {
                    // Already placed this image — skip continuation row.
                    continue;
                }

                let item = &doc.items[entry.item_idx];
                if let DocItem::Image(img) = item {
                    let remaining_image = total_rows - row_in_image;
                    let remaining_screen = max_content_rows - rows_rendered;
                    let visible_rows = remaining_image.min(remaining_screen);

                    let crop_top_px = row_in_image * cell_px_height;
                    // Clamp source height to actual remaining pixels.
                    let src_h = (visible_rows * cell_px_height)
                        .min(img.pixel_height.saturating_sub(crop_top_px));
                    let needs_crop =
                        *row_in_image > 0 || visible_rows < *total_rows;

                    if transmitted.contains(&img.id) {
                        out.push_str(&kitty_place(img, needs_crop, crop_top_px, src_h));
                    } else {
                        out.push_str(&kitty_transmit(img, needs_crop, crop_top_px, src_h));
                        transmitted.insert(img.id);
                    }

                    out.push_str("\r\n");
                    placed_images.insert(entry.item_idx);
                    rows_rendered += visible_rows;
                }
            }
        }
    }

    // Clear remaining rows below content.
    while rows_rendered < max_content_rows {
        out.push_str("\x1b[K\r\n");
        rows_rendered += 1;
    }

    // Status bar.
    let total = layout.len();
    let pct = if total == 0 {
        100
    } else {
        ((top_entry + 1) * 100 / total).min(100)
    };
    out.push_str(&format!(
        "\x1b[{row};1H\x1b[2K\x1b[7m {pct}%\x1b[0m",
        row = screen_rows
    ));

    out
}

/// Emit a `a=p` placement command (no pixel data, uses terminal cache).
///
/// When `crop` is true, the source rectangle is limited via `y=` (pixel
/// offset from top) and `h=` (pixel height to show).  This crops without
/// rescaling — the terminal keeps the same scale factor derived from `c=`.
fn kitty_place(
    img: &crate::document::KittyImage,
    crop: bool,
    crop_top_px: u32,
    src_height_px: u32,
) -> String {
    let cols = img
        .display_cols
        .map(|c| format!(",c={c}"))
        .unwrap_or_default();
    let crop_params = if crop {
        format!(",y={crop_top_px},h={src_height_px}")
    } else {
        String::new()
    };
    format!(
        "\x1b_Ga=p,i={}{}{},q=2;\x1b\\",
        img.id, cols, crop_params
    )
}

/// Emit a `a=T` transmit-and-display command with full pixel data.
///
/// When `crop` is true the source rectangle is limited via `y=` and `h=`
/// (pixel offset and height).  The terminal displays the cropped region at
/// the same scale as the full image — no zoom change.
fn kitty_transmit(
    img: &crate::document::KittyImage,
    crop: bool,
    crop_top_px: u32,
    src_height_px: u32,
) -> String {
    let b64 = STANDARD.encode(&img.rgba_data);
    let cols = img
        .display_cols
        .map(|c| format!(",c={c}"))
        .unwrap_or_default();
    let crop_params = if crop {
        format!(",y={crop_top_px},h={src_height_px}")
    } else {
        String::new()
    };

    let mut out = String::new();
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    let total = chunks.len();

    for (i, chunk) in chunks.iter().enumerate() {
        let chunk_str = std::str::from_utf8(chunk).unwrap();
        let more = if i + 1 < total { 1 } else { 0 };
        if i == 0 {
            out.push_str(&format!(
                "\x1b_Ga=T,f=32,s={w},v={h},i={id}{cols}{crop},q=2,m={more};{chunk_str}\x1b\\",
                w = img.pixel_width,
                h = img.pixel_height,
                id = img.id,
                crop = crop_params,
            ));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk_str}\x1b\\"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocItem, KittyDocument, KittyImage};

    fn make_doc(items: Vec<DocItem>) -> KittyDocument {
        KittyDocument { items }
    }

    #[test]
    fn layout_text_splits_on_newlines() {
        let doc = make_doc(vec![DocItem::Text("line1\nline2\nline3".to_string())]);
        let entries = layout(&doc, 16);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn layout_image_expands_to_rows() {
        let img = KittyImage {
            id: 1,
            rgba_data: vec![0u8; 4 * 10 * 35],
            pixel_width: 10,
            pixel_height: 35,
            display_cols: None,
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        // 35 / 16 = 2.1875 → ceil = 3 → 3 layout entries
        let entries = layout(&doc, 16);
        assert_eq!(entries.len(), 3);
        for (i, e) in entries.iter().enumerate() {
            match &e.kind {
                EntryKind::ImageRow {
                    row_in_image,
                    total_rows,
                } => {
                    assert_eq!(*row_in_image, i as u32);
                    assert_eq!(*total_rows, 3);
                }
                _ => panic!("expected ImageRow"),
            }
        }
    }

    #[test]
    fn layout_image_minimum_one_row() {
        let img = KittyImage {
            id: 2,
            rgba_data: vec![0u8; 4],
            pixel_width: 1,
            pixel_height: 1,
            display_cols: None,
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 16);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn render_frame_contains_status_bar() {
        let doc = make_doc(vec![DocItem::Text("hello".to_string())]);
        let entries = layout(&doc, 16);
        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 24, 16, &mut transmitted);
        assert!(frame.contains("\x1b[7m "));
    }

    #[test]
    fn render_frame_no_screen_clear() {
        let doc = make_doc(vec![DocItem::Text("hello".to_string())]);
        let entries = layout(&doc, 16);
        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 24, 16, &mut transmitted);
        assert!(!frame.contains("\x1b[2J"));
        assert!(frame.contains("a=d,d=a"));
        assert!(frame.contains("\x1b[H"));
    }

    #[test]
    fn render_frame_transmits_then_replaces() {
        let img = KittyImage {
            id: 42,
            rgba_data: vec![255u8; 4 * 4 * 4],
            pixel_width: 4,
            pixel_height: 4,
            display_cols: None,
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 16);
        let mut transmitted = HashSet::new();

        let frame = render_frame(&doc, &entries, 0, 24, 16, &mut transmitted);
        assert!(frame.contains("a=T"));
        assert!(transmitted.contains(&42));

        let frame2 = render_frame(&doc, &entries, 0, 24, 16, &mut transmitted);
        assert!(frame2.contains("a=p,i=42"));
        assert!(!frame2.contains("a=T"));
    }

    #[test]
    fn render_frame_partial_image_top_cropped() {
        // Image is 48px tall / 16px per cell = 3 rows.
        // Start at row 1 (skip first row) → should crop y=16.
        let img = KittyImage {
            id: 7,
            rgba_data: vec![0u8; 4 * 4 * 48],
            pixel_width: 4,
            pixel_height: 48,
            display_cols: Some(10),
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 16);
        assert_eq!(entries.len(), 3); // 3 rows

        let mut transmitted = HashSet::new();
        // Skip the first image row (top_entry=1 → row_in_image=1).
        let frame = render_frame(&doc, &entries, 1, 24, 16, &mut transmitted);
        // Should transmit with y=16 (1 row * 16px) and h=32 (2 visible rows × 16px).
        assert!(frame.contains("y=16"));
        assert!(frame.contains("h=32"));
    }

    #[test]
    fn render_frame_partial_image_bottom_cropped() {
        // Image is 48px tall = 3 rows.  Screen has only 2 content rows (3 - 1 status).
        let img = KittyImage {
            id: 8,
            rgba_data: vec![0u8; 4 * 4 * 48],
            pixel_width: 4,
            pixel_height: 48,
            display_cols: None,
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 16);

        let mut transmitted = HashSet::new();
        // Screen is 3 rows: 2 content + 1 status.  Image needs 3 rows → clipped to 2.
        let frame = render_frame(&doc, &entries, 0, 3, 16, &mut transmitted);
        assert!(frame.contains("h=32"));
    }
}
