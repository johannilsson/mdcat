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

/// Strip ANSI escape sequences (CSI and APC/OSC/PM) from `s`, returning plain text.
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    // CSI sequence: skip until a final byte (ASCII 0x40–0x7E).
                    for c2 in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c2) {
                            break;
                        }
                    }
                }
                Some(intro @ ('_' | ']' | '^' | 'P')) => {
                    // APC / OSC / PM / DCS: skip until String Terminator (\x1b\\) or BEL.
                    let _ = intro;
                    loop {
                        match chars.next() {
                            Some('\x1b') => {
                                chars.next(); // consume '\\'
                                break;
                            }
                            Some('\x07') | None => break,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Build a flat list of terminal rows from a document.
///
/// Text items are split on `\n`; each line becomes one `LayoutEntry`.
/// Image items are expanded into one entry **per terminal row** so that
/// scrolling is row-granular and partial images can be displayed.
///
/// Both `cell_px_width` and `cell_px_height` are needed because images with
/// `display_cols` set are scaled by the terminal — the displayed pixel height
/// depends on the scale factor derived from `display_cols * cell_px_width`.
pub(crate) fn layout(doc: &KittyDocument, cell_px_width: u32, cell_px_height: u32) -> Vec<LayoutEntry> {
    let cell_h = cell_px_height.max(1);
    let cell_w = cell_px_width.max(1);
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
                // When display_cols is set, the Kitty terminal scales the
                // image to that many columns (preserving aspect ratio).
                // We must compute the displayed pixel height after scaling
                // so the layout row count matches what the terminal renders.
                let displayed_height = if let Some(cols) = img.display_cols {
                    let display_width_px = cols as u32 * cell_w;
                    let pw = img.pixel_width.max(1);
                    ((img.pixel_height as u64 * display_width_px as u64) / pw as u64) as u32
                } else {
                    img.pixel_height
                };
                let total_rows = displayed_height.div_ceil(cell_h).max(1);
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
///
/// `search_query`: when non-empty, text lines whose plain content contains the
/// query (case-insensitive) are rendered with reverse-video highlighting.
///
/// `status_text`: appended to the status bar.  Pass the search prompt during
/// input mode, the active query indicator in normal mode, or an empty string.
pub(crate) fn render_frame(
    doc: &KittyDocument,
    layout: &[LayoutEntry],
    top_entry: usize,
    screen_rows: u16,
    cell_px_width: u32,
    cell_px_height: u32,
    transmitted: &mut HashSet<u32>,
    search_query: &str,
    status_text: &str,
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
    let query_lower = search_query.to_lowercase();

    for entry in layout.iter().skip(top_entry) {
        if rows_rendered >= max_content_rows {
            break;
        }

        match &entry.kind {
            EntryKind::Text(line) => {
                let highlighted = !query_lower.is_empty()
                    && strip_ansi(line).to_lowercase().contains(&query_lower);
                if highlighted {
                    out.push_str("\x1b[7m");
                    out.push_str(line);
                    out.push_str("\x1b[0m");
                } else {
                    out.push_str(line);
                }
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

                    // When display_cols is set the terminal scales the image,
                    // so source-pixel coordinates differ from display-pixel
                    // coordinates.  Scale crop values by pixel_width /
                    // (display_cols * cell_px_width) to convert display→source.
                    let (crop_top_px, raw_src_h) = if let Some(cols) = img.display_cols {
                        let display_w = cols as u64 * cell_px_width as u64;
                        let pw = img.pixel_width as u64;
                        let top = (*row_in_image as u64 * cell_px_height as u64 * pw / display_w) as u32;
                        let h = (visible_rows as u64 * cell_px_height as u64 * pw / display_w) as u32;
                        (top, h)
                    } else {
                        (row_in_image * cell_px_height, visible_rows * cell_px_height)
                    };
                    // Clamp source height to actual remaining pixels.
                    let src_h = raw_src_h.min(img.pixel_height.saturating_sub(crop_top_px));
                    let needs_crop =
                        *row_in_image > 0 || visible_rows < *total_rows;

                    // Clear every row the image will occupy.  Kitty
                    // images are virtual overlays — they float on top of
                    // the text grid.  Without clearing, text from a
                    // previous frame bleeds through under the image.
                    for r in 0..visible_rows {
                        let clear_row = rows_rendered + r + 1; // 1-based
                        out.push_str(&format!("\x1b[{clear_row};1H\x1b[K"));
                    }
                    // Return cursor to image start row for placement.
                    let img_row = rows_rendered + 1; // 1-based
                    out.push_str(&format!("\x1b[{img_row};1H"));

                    // Compute effective display columns.  When display_cols
                    // is None (image narrower than terminal), we still need
                    // to anchor the width via `c=` whenever `r=` is used for
                    // cropping — otherwise the terminal may scale the image
                    // to fill the full terminal width.
                    let effective_cols = img.display_cols.unwrap_or_else(|| {
                        let cell_w = cell_px_width.max(1);
                        (img.pixel_width / cell_w).max(1) as u16
                    });

                    if !transmitted.contains(&img.id) {
                        out.push_str(&kitty_transmit(img));
                        transmitted.insert(img.id);
                    }
                    out.push_str(&kitty_place(img, effective_cols, needs_crop, crop_top_px, src_h, visible_rows));

                    placed_images.insert(entry.item_idx);
                    rows_rendered += visible_rows;
                    // Position cursor at the row after the image.
                    let next_row = rows_rendered + 1; // 1-based
                    out.push_str(&format!("\x1b[{next_row};1H"));
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
    if status_text.is_empty() {
        out.push_str(&format!(
            "\x1b[{row};1H\x1b[2K\x1b[7m {pct}%\x1b[0m",
            row = screen_rows
        ));
    } else {
        out.push_str(&format!(
            "\x1b[{row};1H\x1b[2K\x1b[7m {pct}%  {status_text}\x1b[0m",
            row = screen_rows
        ));
    }

    out
}

/// Emit a `a=p` placement command (no pixel data, uses terminal cache).
///
/// `effective_cols` is always provided to anchor the display width.
/// When `crop` is true, the source rectangle is limited via `y=` (pixel
/// offset from top) and `h=` (pixel height to show).  This crops without
/// rescaling — the terminal keeps the same scale factor derived from `c=`.
fn kitty_place(
    img: &crate::document::KittyImage,
    effective_cols: u16,
    crop: bool,
    crop_top_px: u32,
    src_height_px: u32,
    visible_rows: u32,
) -> String {
    let crop_params = if crop {
        format!(",y={crop_top_px},h={src_height_px},r={visible_rows}")
    } else {
        String::new()
    };
    format!(
        "\x1b_Ga=p,i={},c={}{},q=2;\x1b\\",
        img.id, effective_cols, crop_params
    )
}

/// Emit `a=t` commands to transmit pixel data without displaying.
///
/// The image is stored in the terminal's cache under `img.id` and can
/// subsequently be placed (possibly with cropping) via `kitty_place`.
/// Separating transmission from placement avoids visual artifacts during
/// chunked data transfer (the terminal never shows a partially-received
/// image) and ensures the cached image is always the full resolution.
fn kitty_transmit(img: &crate::document::KittyImage) -> String {
    let b64 = STANDARD.encode(&img.rgba_data);

    let mut out = String::new();
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    let total = chunks.len();

    for (i, chunk) in chunks.iter().enumerate() {
        let chunk_str = std::str::from_utf8(chunk).unwrap();
        let more = if i + 1 < total { 1 } else { 0 };
        if i == 0 {
            out.push_str(&format!(
                "\x1b_Ga=t,f=32,s={w},v={h},i={id},q=2,m={more};{chunk_str}\x1b\\",
                w = img.pixel_width,
                h = img.pixel_height,
                id = img.id,
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
        let entries = layout(&doc, 8, 16);
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
        let entries = layout(&doc, 8, 16);
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
        let entries = layout(&doc, 8, 16);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn layout_image_accounts_for_display_cols_scaling() {
        // Image: 3000×1500px (e.g. mermaid at 3x scale), display_cols=200.
        // cell_px_width=9, cell_px_height=18.
        // display_width_px = 200 * 9 = 1800
        // displayed_height = 1500 * 1800 / 3000 = 900
        // total_rows = ceil(900 / 18) = 50
        //
        // Without the fix, raw pixel_height would give ceil(1500/18) = 84 rows.
        let img = KittyImage {
            id: 10,
            rgba_data: vec![0u8; 4 * 3000 * 1500],
            pixel_width: 3000,
            pixel_height: 1500,
            display_cols: Some(200),
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 9, 18);
        assert_eq!(entries.len(), 50);
    }

    #[test]
    fn render_frame_contains_status_bar() {
        let doc = make_doc(vec![DocItem::Text("hello".to_string())]);
        let entries = layout(&doc, 8, 16);
        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "", "");
        assert!(frame.contains("\x1b[7m "));
    }

    #[test]
    fn render_frame_no_screen_clear() {
        let doc = make_doc(vec![DocItem::Text("hello".to_string())]);
        let entries = layout(&doc, 8, 16);
        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "", "");
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
        let entries = layout(&doc, 8, 16);
        let mut transmitted = HashSet::new();

        let frame = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "", "");
        // First frame: transmit (a=t) then place (a=p).
        assert!(frame.contains("a=t"));
        assert!(frame.contains("a=p,i=42"));
        assert!(transmitted.contains(&42));
        // Transmit must come before placement.
        let transmit_pos = frame.find("a=t").unwrap();
        let place_pos = frame.find("a=p").unwrap();
        assert!(transmit_pos < place_pos);

        let frame2 = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "", "");
        // Second frame: placement only (no retransmission).
        assert!(frame2.contains("a=p,i=42"));
        assert!(!frame2.contains("a=t"));
    }

    #[test]
    fn render_frame_partial_image_top_cropped() {
        // Image: 80×48px, display_cols=10, cell 8×16px.
        // Displayed height = 48 * (10*8) / 80 = 48px → ceil(48/16) = 3 rows.
        // Start at row 1 (skip first row) → should crop y=16.
        let img = KittyImage {
            id: 7,
            rgba_data: vec![0u8; 4 * 80 * 48],
            pixel_width: 80,
            pixel_height: 48,
            display_cols: Some(10),
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 8, 16);
        assert_eq!(entries.len(), 3); // 3 rows

        let mut transmitted = HashSet::new();
        // Skip the first image row (top_entry=1 → row_in_image=1).
        let frame = render_frame(&doc, &entries, 1, 24, 8, 16, &mut transmitted, "", "");
        // Should transmit with y=16 (1 row * 16px), h=32 (2 visible rows × 16px),
        // and r=2 (2 visible display rows).
        assert!(frame.contains("y=16"));
        assert!(frame.contains("h=32"));
        assert!(frame.contains("r=2"));
    }

    #[test]
    fn render_frame_scaled_image_crop_coordinates() {
        // Image: 3000×1500px, display_cols=200, cell 9×18px.
        // Scale factor = display_w / pixel_w = 1800 / 3000 = 0.6
        // Inverse scale = 3000 / 1800 = 5/3
        // Displayed height = 1500 * 1800 / 3000 = 900px → 50 rows.
        //
        // Scrolling to row 1:
        //   crop_top = 1 * 18 * 3000 / 1800 = 30
        //   src_h    = 49 * 18 * 3000 / 1800 = 1470
        // (screen 51 rows → 50 content rows → 49 visible from row 1)
        let img = KittyImage {
            id: 20,
            rgba_data: vec![0u8; 4 * 3000 * 1500],
            pixel_width: 3000,
            pixel_height: 1500,
            display_cols: Some(200),
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 9, 18);
        assert_eq!(entries.len(), 50);

        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 1, 51, 9, 18, &mut transmitted, "", "");
        assert!(frame.contains("y=30"), "expected y=30 in frame");
        assert!(frame.contains("h=1470"), "expected h=1470 in frame");
        assert!(frame.contains("r=49"), "expected r=49 in frame");
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
        let entries = layout(&doc, 8, 16);

        let mut transmitted = HashSet::new();
        // Screen is 3 rows: 2 content + 1 status.  Image needs 3 rows → clipped to 2.
        let frame = render_frame(&doc, &entries, 0, 3, 8, 16, &mut transmitted, "", "");
        assert!(frame.contains("h=32"));
        assert!(frame.contains("r=2"));
    }

    #[test]
    fn render_frame_clears_rows_under_image() {
        // Text (5 lines) followed by an image (3 rows at 48px / 16px cell).
        // With a 10-row screen (9 content + 1 status), the image starts at
        // row 6 and occupies rows 6–8.  Those rows must be cleared before
        // the image is placed so old text from a previous frame doesn't
        // bleed through the Kitty virtual overlay.
        let img = KittyImage {
            id: 99,
            rgba_data: vec![0u8; 4 * 4 * 48],
            pixel_width: 4,
            pixel_height: 48,
            display_cols: None,
        };
        let doc = make_doc(vec![
            DocItem::Text("a\nb\nc\nd\ne".to_string()),
            DocItem::Image(img),
        ]);
        let entries = layout(&doc, 8, 16);
        // 5 text rows + 3 image rows = 8 entries.
        assert_eq!(entries.len(), 8);

        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 10, 8, 16, &mut transmitted, "", "");

        // Image starts at content row 5 → terminal row 6 (1-based).
        // Rows 6, 7, 8 must each have a clear-line sequence (\x1b[K)
        // emitted BEFORE the image placement escape.
        let place_pos = frame.find("a=p").expect("expected image placement");
        for row in 6..=8u32 {
            let clear = format!("\x1b[{row};1H\x1b[K");
            let clear_pos = frame.find(&clear).unwrap_or_else(|| {
                panic!("expected clear sequence for row {row}: {clear:?}")
            });
            assert!(
                clear_pos < place_pos,
                "clear for row {row} must come before image placement"
            );
        }
    }

    #[test]
    fn render_frame_natural_size_image_always_has_cols() {
        // Image: 80×48px, display_cols=None (natural size), cell 8×16.
        // natural_cols = 80 / 8 = 10.
        // When cropped (partial visibility), c=10 must be included
        // to prevent the terminal from scaling the image to full width.
        let img = KittyImage {
            id: 50,
            rgba_data: vec![0u8; 4 * 80 * 48],
            pixel_width: 80,
            pixel_height: 48,
            display_cols: None,
        };
        let doc = make_doc(vec![DocItem::Image(img)]);
        let entries = layout(&doc, 8, 16);
        assert_eq!(entries.len(), 3); // 48 / 16 = 3 rows

        let mut transmitted = HashSet::new();
        // Screen has 2 content rows → image is bottom-cropped to 2 rows.
        let frame = render_frame(&doc, &entries, 0, 3, 8, 16, &mut transmitted, "", "");
        assert!(frame.contains("c=10"), "expected c=10 for natural-size image crop");
        assert!(frame.contains("r=2"), "expected r=2 for bottom crop");

        // Full image visible (no crop) — c= should still be present.
        let frame2 = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "", "");
        assert!(frame2.contains("c=10"), "expected c=10 even without crop");
    }

    #[test]
    fn render_frame_highlights_search_match() {
        let doc = make_doc(vec![DocItem::Text("foo bar\nbaz qux".to_string())]);
        let entries = layout(&doc, 8, 16);
        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "bar", "");
        // The matching line ("foo bar") should be wrapped in reverse-video.
        assert!(frame.contains("\x1b[7mfoo bar\x1b[0m"), "expected highlighted match line");
        // The non-matching line should not be highlighted.
        assert!(!frame.contains("\x1b[7mbaz qux"));
    }

    #[test]
    fn render_frame_no_highlight_when_no_query() {
        let doc = make_doc(vec![DocItem::Text("hello world".to_string())]);
        let entries = layout(&doc, 8, 16);
        let mut transmitted = HashSet::new();
        let frame = render_frame(&doc, &entries, 0, 24, 8, 16, &mut transmitted, "", "");
        assert!(!frame.contains("\x1b[7mhello world"));
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        assert_eq!(strip_ansi("\x1b[1mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("\x1b[38;5;200mcolor\x1b[0m"), "color");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn strip_ansi_removes_apc_sequences() {
        // APC used by Kitty graphics protocol
        let s = format!("text\x1b_Ga=t;\x1b\\more");
        assert_eq!(strip_ansi(&s), "textmore");
    }
}
