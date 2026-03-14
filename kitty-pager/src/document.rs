/// A pre-transmitted Kitty graphics protocol image.
pub struct KittyImage {
    /// Unique image ID used for caching and re-placement.
    pub id: u32,
    /// Raw RGBA8 pixel data (row-major).
    pub rgba_data: Vec<u8>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// If `Some(n)`, scale the image to exactly n terminal columns.
    /// `None` lets the terminal display at natural pixel size.
    pub display_cols: Option<u16>,
}

/// A single item in a pageable document.
pub enum DocItem {
    /// ANSI-formatted text (may contain newlines and escape sequences).
    Text(String),
    /// An inline image.
    Image(KittyImage),
}

/// A document that can be displayed by the Kitty pager.
pub struct KittyDocument {
    pub items: Vec<DocItem>,
}
