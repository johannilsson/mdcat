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

/// Like [`show`], but `refresh` is polled while paging and any new content it
/// returns replaces the text on screen.
///
/// The pager runs on its own thread so this one stays free to call `refresh`
/// (the render config isn't `Send`). `dynamic_paging` is used because the
/// static variant exits early when the content fits the screen.
pub fn show_live(content: String, mut refresh: impl FnMut() -> Option<String>) -> Result<()> {
    let pager = minus::Pager::new();
    pager.set_text(content)?;
    pager.set_prompt("mdcat")?;

    std::thread::scope(|scope| {
        let paging = scope.spawn({
            let pager = pager.clone();
            move || minus::dynamic_paging(pager)
        });
        while !paging.is_finished() {
            std::thread::sleep(crate::watch::POLL_INTERVAL);
            if let Some(text) = refresh() {
                // Fails only once the pager has quit.
                let _ = pager.set_text(text);
            }
        }
        paging
            .join()
            .map_err(|_| anyhow::anyhow!("pager thread panicked"))?
            .map_err(Into::into)
    })
}
