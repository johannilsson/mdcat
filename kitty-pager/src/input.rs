use std::collections::HashSet;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::document::KittyDocument;
use crate::renderer::{layout, render_frame};
use crate::PagerOptions;

/// How long to wait for input before asking for a refreshed document.
const REFRESH_INTERVAL: Duration = Duration::from_millis(250);

type Refresh<'a> = Option<&'a mut dyn FnMut() -> Option<KittyDocument>>;

/// Run the interactive pager event loop.
pub(crate) fn run_pager(mut doc: KittyDocument, opts: &PagerOptions, refresh: Refresh) -> Result<()> {
    let cell_w = opts.cell_pixel_width.max(1);
    let cell_h = opts.cell_pixel_height.max(1);
    let mut entries = layout(&doc, cell_w, cell_h);
    if entries.is_empty() {
        return Ok(());
    }

    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    write!(stdout, "\x1b[?7l")?;
    stdout.flush()?;

    let result = event_loop(&mut doc, &mut entries, opts, &mut stdout, refresh);

    let _ = write!(stdout, "\x1b[?7h");
    let _ = stdout.flush();
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();

    result
}

fn event_loop(
    doc: &mut KittyDocument,
    entries: &mut Vec<crate::renderer::LayoutEntry>,
    opts: &PagerOptions,
    stdout: &mut impl Write,
    mut refresh: Refresh,
) -> Result<()> {
    let mut top_entry = 0usize;
    let mut transmitted: HashSet<u32> = HashSet::new();
    let mut screen_rows = opts.term_height;
    let cell_w = opts.cell_pixel_width.max(1);
    let cell_h = opts.cell_pixel_height.max(1);

    let frame = render_frame(doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted);
    write!(stdout, "{}", frame)?;
    stdout.flush()?;

    loop {
        if !event::poll(REFRESH_INTERVAL)? {
            if let Some(new_doc) = refresh.as_mut().and_then(|r| r()) {
                *doc = new_doc;
                *entries = layout(doc, cell_w, cell_h);
                top_entry = top_entry.min(entries.len().saturating_sub(1));
                // The new document numbers its images from scratch, so the
                // terminal's copies of the old ones must not be reused.
                write!(stdout, "\x1b_Ga=d,d=A,q=2;\x1b\\")?;
                transmitted.clear();
                let frame = render_frame(
                    doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }
            continue;
        }
        match event::read()? {
            Event::Key(KeyEvent { code, modifiers, .. }) => {
                let max_top = entries.len().saturating_sub(1);
                let half_page = (screen_rows as usize).saturating_sub(2).max(1) / 2;
                let full_page = (screen_rows as usize).saturating_sub(2).max(1);

                match (code, modifiers) {
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Char('Q'), _)
                    | (KeyCode::Esc, _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                    (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                        top_entry = (top_entry + 1).min(max_top);
                    }
                    (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                        top_entry = top_entry.saturating_sub(1);
                    }
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                        top_entry = (top_entry + half_page).min(max_top);
                    }
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                        top_entry = top_entry.saturating_sub(half_page);
                    }
                    (KeyCode::PageDown, _) => {
                        top_entry = (top_entry + full_page).min(max_top);
                    }
                    (KeyCode::PageUp, _) => {
                        top_entry = top_entry.saturating_sub(full_page);
                    }
                    (KeyCode::Home, _) | (KeyCode::Char('g'), _) => {
                        top_entry = 0;
                    }
                    (KeyCode::End, _) | (KeyCode::Char('G'), _) => {
                        top_entry = max_top;
                    }
                    _ => continue,
                }

                let frame = render_frame(
                    doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }

            Event::Resize(_new_cols, new_rows) => {
                screen_rows = new_rows;
                *entries = layout(doc, cell_w, cell_h);
                let max_top = entries.len().saturating_sub(1);
                top_entry = top_entry.min(max_top);

                let frame = render_frame(
                    doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }

            _ => {}
        }
    }

    Ok(())
}
