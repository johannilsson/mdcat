# kitty-pager

A minimal terminal pager with [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) support for inline images. Built as a library crate so it can be embedded in other tools.

## How it works

The caller builds a `KittyDocument` — a sequence of `DocItem::Text` (ANSI-formatted strings) and `DocItem::Image` (raw RGBA pixel data) items — and hands it to `page()`.

The pager then:

1. **Lays out** the document into terminal rows. Text is split on newlines; images are expanded into one row per cell-height slice so scrolling is row-granular.
2. **Renders frames** using the Kitty graphics protocol. Images are transmitted once (`a=T`) and re-placed from cache (`a=p`) on subsequent frames. Partial images (top- or bottom-cropped) use source-rectangle cropping (`y=`, `h=`) to display a pixel sub-region without rescaling.
3. **Runs an event loop** (via crossterm) in the alternate screen with raw mode. Flicker-free redraws: placements are deleted (`a=d,d=a`), cursor is homed, and lines are overwritten with clear-to-EOL — no full screen clear.

## Key bindings

| Key | Action |
|-----|--------|
| `j` / Down | Scroll down one row |
| `k` / Up | Scroll up one row |
| `f` / Space / PageDown | Scroll down one page |
| `b` / PageUp | Scroll up one page |
| `g` / Home | Go to top |
| `G` / End | Go to bottom |
| `q` / `Q` / Esc / Ctrl-C | Quit |

## Supported terminals

Works with terminals that implement the Kitty graphics protocol:

- **Kitty**
- **Ghostty**

## Current limitations

- **No search.** There is no text search within the pager.
- **No mouse support.** Scrolling is keyboard-only.
- **No horizontal scrolling.** Long lines are clipped by the terminal (line-wrap is disabled).
- **Images are protocol-specific.** Only the Kitty graphics protocol is supported; iTerm2/Sixel images will not work.
- **No reflowing.** Text is pre-rendered before entering the pager. Resizing the terminal re-layouts image row counts but does not reflow text.
- **Cell pixel size detection.** Uses CSI 16t to query cell dimensions. Falls back to 8x16 if the terminal doesn't respond, which may cause incorrect image row calculations.

## Usage

```rust
use kitty_pager::{KittyDocument, DocItem, KittyImage, PagerOptions, page};

let doc = KittyDocument {
    items: vec![
        DocItem::Text("# Hello\n\nSome text.".to_string()),
        DocItem::Image(KittyImage {
            id: 1,
            rgba_data: pixel_bytes,
            pixel_width: 200,
            pixel_height: 150,
            display_cols: Some(40),
        }),
        DocItem::Text("More text after the image.".to_string()),
    ],
};

page(doc, PagerOptions::default())?;
```

## Dependencies

- `crossterm` — raw mode, alternate screen, key events
- `base64` — encoding image data for the Kitty protocol
- `anyhow` — error handling
- `libc` (unix only) — terminal pixel size query via CSI 16t
