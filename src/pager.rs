use anyhow::Result;

/// Display rendered content in an embedded pager (minus).
/// The pager handles ANSI escape sequences correctly, including terminal
/// graphics protocols (iTerm2, Kitty) which would be stripped by `less`.
pub fn show(content: String) -> Result<()> {
    let pager = minus::Pager::new();

    pager.set_text(content)?;
    pager.set_prompt("mdcat")?;

    // page_all is the static (blocking) variant — use this when all content
    // is available upfront. Supports q/esc/Q to exit, vim-style j/k scrolling.
    minus::page_all(pager)?;
    Ok(())
}
