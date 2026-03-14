mod document;
mod input;
mod renderer;
pub mod terminal;

pub use document::{DocItem, KittyDocument, KittyImage};

/// Options for the Kitty pager.
pub struct PagerOptions {
    /// Terminal width in columns.
    pub term_width: u16,
    /// Terminal height in rows.
    pub term_height: u16,
    /// Width of one terminal cell in pixels.
    pub cell_pixel_width: u32,
    /// Height of one terminal cell in pixels.
    pub cell_pixel_height: u32,
}

impl Default for PagerOptions {
    fn default() -> Self {
        let (cell_px_w, cell_px_h) = terminal::query_cell_pixel_size();
        let (term_w, term_h) = crossterm::terminal::size().unwrap_or((80, 24));
        PagerOptions {
            term_width: term_w,
            term_height: term_h,
            cell_pixel_width: cell_px_w,
            cell_pixel_height: cell_px_h,
        }
    }
}

/// Display `doc` in an interactive fullscreen pager.
///
/// Returns when the user presses `q`, `Q`, or `Esc`.
pub fn page(doc: KittyDocument, opts: PagerOptions) -> anyhow::Result<()> {
    input::run_pager(&doc, &opts)
}
