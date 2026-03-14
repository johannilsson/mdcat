use std::collections::HashSet;
use std::io::Write;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::document::KittyDocument;
use crate::renderer::{layout, render_frame};
use crate::PagerOptions;

/// Run the interactive pager event loop.
pub(crate) fn run_pager(doc: &KittyDocument, opts: &PagerOptions) -> Result<()> {
    let cell_h = opts.cell_pixel_height.max(1);
    let mut entries = layout(doc, cell_h);
    if entries.is_empty() {
        return Ok(());
    }

    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    write!(stdout, "\x1b[?7l")?;
    stdout.flush()?;

    let result = event_loop(doc, &mut entries, opts, &mut stdout);

    let _ = write!(stdout, "\x1b[?7h");
    let _ = stdout.flush();
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();

    result
}

fn event_loop(
    doc: &KittyDocument,
    entries: &mut Vec<crate::renderer::LayoutEntry>,
    opts: &PagerOptions,
    stdout: &mut impl Write,
) -> Result<()> {
    let mut top_entry = 0usize;
    let mut transmitted: HashSet<u32> = HashSet::new();
    let mut screen_rows = opts.term_height;
    let cell_h = opts.cell_pixel_height.max(1);

    let frame = render_frame(doc, entries, top_entry, screen_rows, cell_h, &mut transmitted);
    write!(stdout, "{}", frame)?;
    stdout.flush()?;

    loop {
        match event::read()? {
            Event::Key(KeyEvent { code, modifiers, .. }) => {
                let page_size = (screen_rows as usize).saturating_sub(2).max(1);
                let max_top = entries.len().saturating_sub(1);

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
                    (KeyCode::Char('f'), _)
                    | (KeyCode::PageDown, _)
                    | (KeyCode::Char(' '), _) => {
                        top_entry = (top_entry + page_size).min(max_top);
                    }
                    (KeyCode::Char('b'), _) | (KeyCode::PageUp, _) => {
                        top_entry = top_entry.saturating_sub(page_size);
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
                    doc, entries, top_entry, screen_rows, cell_h, &mut transmitted,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }

            Event::Resize(_new_cols, new_rows) => {
                screen_rows = new_rows;
                *entries = layout(doc, cell_h);
                let max_top = entries.len().saturating_sub(1);
                top_entry = top_entry.min(max_top);

                let frame = render_frame(
                    doc, entries, top_entry, screen_rows, cell_h, &mut transmitted,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }

            _ => {}
        }
    }

    Ok(())
}
