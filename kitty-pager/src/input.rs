use std::collections::HashSet;
use std::io::Write;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::document::KittyDocument;
use crate::renderer::{layout, render_frame, strip_ansi, EntryKind, LayoutEntry};
use crate::PagerOptions;

// ── Search types ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum SearchDirection {
    Forward,
    Backward,
}

impl SearchDirection {
    fn prefix(self) -> char {
        match self {
            SearchDirection::Forward => '/',
            SearchDirection::Backward => '?',
        }
    }

    fn reversed(self) -> Self {
        match self {
            SearchDirection::Forward => SearchDirection::Backward,
            SearchDirection::Backward => SearchDirection::Forward,
        }
    }
}

struct SearchState {
    query: String,
    /// Sorted layout-entry indices whose plain text contains the query.
    matches: Vec<usize>,
    /// Current position within `matches`.
    current: usize,
    /// Direction of the search that set `query`; `n` repeats it, `N` reverses it.
    direction: SearchDirection,
}

enum InputMode {
    Normal,
    SearchInput {
        buffer: String,
        direction: SearchDirection,
    },
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return the layout-entry indices (in order) whose text contains `query`
/// (case-insensitive).  Image rows are never matched.
fn find_matches(entries: &[LayoutEntry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let lower = query.to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            if let EntryKind::Text(line) = &entry.kind {
                if strip_ansi(line).to_lowercase().contains(&lower) {
                    Some(i)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

/// Index into `matches` of the first match to jump to when a search starts
/// with `top_entry` at the top of the screen.  Both directions include the top
/// line itself.  Wraps around when nothing lies in the search direction.
/// `matches` must be non-empty.
fn initial_match(matches: &[usize], top_entry: usize, direction: SearchDirection) -> usize {
    match direction {
        SearchDirection::Forward => matches.iter().position(|&m| m >= top_entry).unwrap_or(0),
        SearchDirection::Backward => matches
            .iter()
            .rposition(|&m| m <= top_entry)
            .unwrap_or(matches.len() - 1),
    }
}

/// The query to run when the search prompt is submitted with `buffer`.  Like
/// `less`, an empty buffer repeats the previous query; with no previous search
/// there is nothing to run.
fn submitted_query(buffer: &str, previous: &Option<SearchState>) -> Option<String> {
    if buffer.is_empty() {
        previous.as_ref().map(|s| s.query.clone())
    } else {
        Some(buffer.to_string())
    }
}

/// Step `current` one match in `direction`, wrapping around.  `len` must be
/// non-zero.
fn step_match(current: usize, len: usize, direction: SearchDirection) -> usize {
    match direction {
        SearchDirection::Forward => (current + 1) % len,
        SearchDirection::Backward => current.checked_sub(1).unwrap_or(len - 1),
    }
}

/// Build the string to append to the status bar based on the current mode and
/// search state.  Empty string means nothing extra is shown.
fn status_suffix(mode: &InputMode, search: &Option<SearchState>) -> String {
    match mode {
        InputMode::SearchInput { buffer, direction } => {
            format!("{}{buffer}_", direction.prefix())
        }
        InputMode::Normal => match search {
            Some(s) if s.matches.is_empty() => {
                format!("{}{} [No matches]", s.direction.prefix(), s.query)
            }
            Some(s) => format!(
                "{}{} [{}/{}]",
                s.direction.prefix(),
                s.query,
                s.current + 1,
                s.matches.len()
            ),
            None => String::new(),
        },
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run the interactive pager event loop.
pub(crate) fn run_pager(doc: &KittyDocument, opts: &PagerOptions) -> Result<()> {
    let cell_w = opts.cell_pixel_width.max(1);
    let cell_h = opts.cell_pixel_height.max(1);
    let mut entries = layout(doc, cell_w, cell_h);
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

// ── Event loop ────────────────────────────────────────────────────────────────

fn event_loop(
    doc: &KittyDocument,
    entries: &mut Vec<LayoutEntry>,
    opts: &PagerOptions,
    stdout: &mut impl Write,
) -> Result<()> {
    let mut top_entry = 0usize;
    let mut transmitted: HashSet<u32> = HashSet::new();
    let mut screen_rows = opts.term_height;
    let cell_w = opts.cell_pixel_width.max(1);
    let cell_h = opts.cell_pixel_height.max(1);
    let mut mode = InputMode::Normal;
    let mut search: Option<SearchState> = None;

    // Initial render.
    {
        let sq = search.as_ref().map_or("", |s| s.query.as_str());
        let st = status_suffix(&mode, &search);
        let frame = render_frame(doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted, sq, &st);
        write!(stdout, "{}", frame)?;
        stdout.flush()?;
    }

    loop {
        match event::read()? {
            Event::Key(KeyEvent { code, modifiers, .. }) => {
                match mode {
                    // ── Search-input mode: capture the query string ──────────
                    InputMode::SearchInput { ref mut buffer, direction } => {
                        match code {
                            KeyCode::Esc => {
                                mode = InputMode::Normal;
                            }
                            KeyCode::Enter => {
                                if let Some(query) = submitted_query(buffer, &search) {
                                    let matches = find_matches(entries, &query);
                                    let current = if matches.is_empty() {
                                        0
                                    } else {
                                        initial_match(&matches, top_entry, direction)
                                    };
                                    if !matches.is_empty() {
                                        top_entry = matches[current];
                                    }
                                    search = Some(SearchState { query, matches, current, direction });
                                }
                                mode = InputMode::Normal;
                            }
                            KeyCode::Backspace => {
                                buffer.pop();
                            }
                            KeyCode::Char(c)
                                if modifiers == KeyModifiers::NONE
                                    || modifiers == KeyModifiers::SHIFT =>
                            {
                                buffer.push(c);
                            }
                            _ => continue,
                        }
                    }

                    // ── Normal mode: scrolling, search initiation, quit ──────
                    InputMode::Normal => {
                        let max_top = entries.len().saturating_sub(1);
                        let half_page = (screen_rows as usize).saturating_sub(2).max(1) / 2;
                        let full_page = (screen_rows as usize).saturating_sub(2).max(1);

                        match (code, modifiers) {
                            (KeyCode::Char('q'), _)
                            | (KeyCode::Char('Q'), _)
                            | (KeyCode::Esc, _)
                            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                            // Search initiation
                            (KeyCode::Char('/'), _) => {
                                mode = InputMode::SearchInput {
                                    buffer: String::new(),
                                    direction: SearchDirection::Forward,
                                };
                            }
                            (KeyCode::Char('?'), _) => {
                                mode = InputMode::SearchInput {
                                    buffer: String::new(),
                                    direction: SearchDirection::Backward,
                                };
                            }

                            // Search navigation
                            (KeyCode::Char(c @ ('n' | 'N')), _) => {
                                if let Some(ref mut s) = search {
                                    if !s.matches.is_empty() {
                                        let dir = if c == 'n' { s.direction } else { s.direction.reversed() };
                                        s.current = step_match(s.current, s.matches.len(), dir);
                                        top_entry = s.matches[s.current];
                                    }
                                } else {
                                    continue;
                                }
                            }

                            // Scrolling
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
                    }
                }

                // Re-render after any handled key.
                let sq = search.as_ref().map_or("", |s| s.query.as_str());
                let st = status_suffix(&mode, &search);
                let frame = render_frame(
                    doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted, sq, &st,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }

            Event::Resize(_new_cols, new_rows) => {
                screen_rows = new_rows;
                *entries = layout(doc, cell_w, cell_h);
                let max_top = entries.len().saturating_sub(1);
                top_entry = top_entry.min(max_top);
                // Re-run search on new layout so match indices stay valid.
                if let Some(ref mut s) = search {
                    s.matches = find_matches(entries, &s.query);
                    if !s.matches.is_empty() {
                        s.current = s.current.min(s.matches.len() - 1);
                    }
                }

                let sq = search.as_ref().map_or("", |s| s.query.as_str());
                let st = status_suffix(&mode, &search);
                let frame = render_frame(
                    doc, entries, top_entry, screen_rows, cell_w, cell_h, &mut transmitted, sq, &st,
                );
                write!(stdout, "{}", frame)?;
                stdout.flush()?;
            }

            _ => {}
        }
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocItem, KittyDocument, KittyImage};
    use crate::renderer::layout;

    fn text_doc(text: &str) -> KittyDocument {
        KittyDocument { items: vec![DocItem::Text(text.to_string())] }
    }

    fn image_doc() -> KittyDocument {
        KittyDocument {
            items: vec![DocItem::Image(KittyImage {
                id: 1,
                rgba_data: vec![0u8; 4 * 8 * 8],
                pixel_width: 8,
                pixel_height: 8,
                display_cols: None,
            })],
        }
    }

    #[test]
    fn find_matches_text_entries() {
        let doc = text_doc("hello world\nfoo bar\nhello again");
        let entries = layout(&doc, 8, 16);
        let matches = find_matches(&entries, "hello");
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn find_matches_ignores_images() {
        let doc = image_doc();
        let entries = layout(&doc, 8, 16);
        let matches = find_matches(&entries, "anything");
        assert!(matches.is_empty());
    }

    #[test]
    fn find_matches_case_insensitive() {
        let doc = text_doc("Hello World");
        let entries = layout(&doc, 8, 16);
        assert!(!find_matches(&entries, "hello").is_empty());
        assert!(!find_matches(&entries, "WORLD").is_empty());
    }

    #[test]
    fn find_matches_no_match() {
        let doc = text_doc("foo bar");
        let entries = layout(&doc, 8, 16);
        assert!(find_matches(&entries, "xyz").is_empty());
    }

    #[test]
    fn find_matches_empty_query_returns_empty() {
        let doc = text_doc("foo bar");
        let entries = layout(&doc, 8, 16);
        assert!(find_matches(&entries, "").is_empty());
    }

    #[test]
    fn status_suffix_search_input_forward() {
        let mode = InputMode::SearchInput {
            buffer: "foo".to_string(),
            direction: SearchDirection::Forward,
        };
        assert_eq!(status_suffix(&mode, &None), "/foo_");
    }

    #[test]
    fn status_suffix_search_input_backward() {
        let mode = InputMode::SearchInput {
            buffer: "bar".to_string(),
            direction: SearchDirection::Backward,
        };
        assert_eq!(status_suffix(&mode, &None), "?bar_");
    }

    #[test]
    fn status_suffix_normal_with_matches() {
        let search = Some(SearchState {
            query: "foo".to_string(),
            matches: vec![0, 5, 10],
            current: 1,
            direction: SearchDirection::Forward,
        });
        assert_eq!(status_suffix(&InputMode::Normal, &search), "/foo [2/3]");
    }

    #[test]
    fn status_suffix_normal_no_matches() {
        let search = Some(SearchState {
            query: "xyz".to_string(),
            matches: vec![],
            current: 0,
            direction: SearchDirection::Forward,
        });
        assert_eq!(status_suffix(&InputMode::Normal, &search), "/xyz [No matches]");
    }

    #[test]
    fn status_suffix_backward_search_shows_question_mark() {
        let search = Some(SearchState {
            query: "foo".to_string(),
            matches: vec![3],
            current: 0,
            direction: SearchDirection::Backward,
        });
        assert_eq!(status_suffix(&InputMode::Normal, &search), "?foo [1/1]");
        let none = Some(SearchState {
            query: "foo".to_string(),
            matches: vec![],
            current: 0,
            direction: SearchDirection::Backward,
        });
        assert_eq!(status_suffix(&InputMode::Normal, &none), "?foo [No matches]");
    }

    #[test]
    fn initial_match_forward_starts_at_top_line() {
        let matches = [2, 5, 9];
        assert_eq!(initial_match(&matches, 0, SearchDirection::Forward), 0);
        assert_eq!(initial_match(&matches, 5, SearchDirection::Forward), 1);
        assert_eq!(initial_match(&matches, 6, SearchDirection::Forward), 2);
        // Nothing below: wrap to the first match.
        assert_eq!(initial_match(&matches, 10, SearchDirection::Forward), 0);
    }

    #[test]
    fn initial_match_backward() {
        let matches = [2, 5, 9];
        assert_eq!(initial_match(&matches, 10, SearchDirection::Backward), 2);
        assert_eq!(initial_match(&matches, 6, SearchDirection::Backward), 1);
        // Nothing above: wrap to the last match.
        assert_eq!(initial_match(&matches, 1, SearchDirection::Backward), 2);
    }

    #[test]
    fn step_match_follows_direction_and_wraps() {
        assert_eq!(step_match(0, 3, SearchDirection::Forward), 1);
        assert_eq!(step_match(2, 3, SearchDirection::Forward), 0);
        assert_eq!(step_match(1, 3, SearchDirection::Backward), 0);
        assert_eq!(step_match(0, 3, SearchDirection::Backward), 2);
    }

    #[test]
    fn submitted_query_uses_buffer() {
        assert_eq!(submitted_query("foo", &None), Some("foo".to_string()));
    }

    #[test]
    fn submitted_query_empty_repeats_previous() {
        let prev = Some(SearchState {
            query: "bar".to_string(),
            matches: vec![],
            current: 0,
            direction: SearchDirection::Forward,
        });
        assert_eq!(submitted_query("", &prev), Some("bar".to_string()));
        assert_eq!(submitted_query("", &None), None);
    }

    #[test]
    fn search_direction_reversed() {
        assert_eq!(SearchDirection::Forward.reversed(), SearchDirection::Backward);
        assert_eq!(SearchDirection::Backward.reversed(), SearchDirection::Forward);
    }

    #[test]
    fn status_suffix_normal_no_search() {
        assert_eq!(status_suffix(&InputMode::Normal, &None), "");
    }
}
