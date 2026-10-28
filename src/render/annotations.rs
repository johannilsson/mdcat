//! Folio annotations drawn over the rendered document (`--annotations`).
//!
//! Folio's engine anchors each pending annotation to a byte range of the raw
//! markdown; comrak's source positions say which bytes a text node came from.
//! Where the two meet, the text is styled the way the Folio app shows it, and
//! the annotation's comment and replies are printed after its paragraph.
//! Read-only: accepting and rejecting stay with `folio accept` / `folio reject`.

use crate::render::{render_document, Config};
use folio::sidecar::{Annotation, AnnotationKind, Sidecar};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

const STRIKE_RED: &str = "\x1b[9;31m";
const STRIKE_RED_OFF: &str = "\x1b[29m\x1b[39m";
const GREEN: &str = "\x1b[32m";
const GREEN_OFF: &str = "\x1b[39m";
const PURPLE_BG: &str = "\x1b[48;5;54m";
const PURPLE_BG_OFF: &str = "\x1b[49m";
const PURPLE: &str = "\x1b[38;5;141m";
/// The terminal's own foreground, what coloured spans go back to by default.
pub const DEFAULT_FG: &str = "\x1b[39m";

/// A pending annotation and where it sits in the raw markdown.
pub struct Mark {
    pub number: usize,
    pub annotation: Annotation,
    /// Byte range in the markdown; `None` when it no longer anchors.
    pub range: Option<(usize, usize)>,
}

/// The annotations of one document, and which of them have been drawn so far.
pub struct Overlay {
    pub marks: Vec<Mark>,
    markdown: String,
    line_starts: Vec<usize>,
    /// Marks drawn inline whose notes are waiting for the end of the paragraph.
    pending_notes: RefCell<BTreeSet<usize>>,
    /// Marks drawn inline anywhere (so a point mark is drawn once).
    drawn: RefCell<BTreeSet<usize>>,
    /// Marks in a heading or table: not drawn, so their notes show the change.
    in_block: RefCell<BTreeSet<usize>>,
}

impl Overlay {
    /// The pending annotations in `file`'s sidecar, or `None` when it has none.
    pub fn load(file: &Path, markdown: &str) -> anyhow::Result<Option<Self>> {
        let sidecar_path = folio::commands::folio_path(file);
        if !sidecar_path.exists() {
            return Ok(None);
        }
        let sidecar = Sidecar::load(&sidecar_path)?;
        Ok(Self::new(markdown, sidecar.annotations))
    }

    pub fn new(markdown: &str, annotations: Vec<Annotation>) -> Option<Self> {
        let mut marks: Vec<Mark> = annotations
            .into_iter()
            .filter(|a| !a.resolved)
            .map(|a| Mark { number: 0, range: a.anchor(markdown), annotation: a })
            .collect();
        if marks.is_empty() {
            return None;
        }
        // Number in document order, unanchored last (as the app parks them).
        marks.sort_by_key(|m| m.range.map(|r| r.0).unwrap_or(usize::MAX));
        for (i, m) in marks.iter_mut().enumerate() {
            m.number = i + 1;
        }
        let line_starts = std::iter::once(0)
            .chain(markdown.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Some(Self {
            marks,
            markdown: markdown.to_string(),
            line_starts,
            pending_notes: RefCell::new(BTreeSet::new()),
            drawn: RefCell::new(BTreeSet::new()),
            in_block: RefCell::new(BTreeSet::new()),
        })
    }

    /// The byte range of a comrak source position (1-based lines and byte
    /// columns, end inclusive).
    pub fn byte_range(&self, sp: comrak::nodes::Sourcepos) -> Option<(usize, usize)> {
        let start = self.line_starts.get(sp.start.line.checked_sub(1)?)? + sp.start.column.checked_sub(1)?;
        let end = self.line_starts.get(sp.end.line.checked_sub(1)?)? + sp.end.column;
        (start <= end).then_some((start, end))
    }

    /// `text`, which comrak parsed from `markdown[start..end]`, with the
    /// annotations that fall in that range drawn into it. `fg` is the colour
    /// to go back to after a coloured span (a heading's, or `DEFAULT_FG`).
    pub fn style_text(&self, text: &str, start: usize, end: usize, fg: &str) -> String {
        let source = self.markdown.get(start..end).unwrap_or("");
        // Where each source byte lands in `text`; `None` when they can't be
        // lined up, and then the whole node is styled instead.
        let align = align(text, source);
        let to_text = |raw: usize| match &align {
            Some(map) => map.iter().find(|(_, src)| *src >= raw - start).map(|(t, _)| *t).unwrap_or(text.len()),
            None => 0,
        };
        let mut cuts: Vec<(usize, Event)> = Vec::new();
        for m in &self.marks {
            let Some((from, to)) = m.range else { continue };
            if from == to {
                // A point (insert, or a comment with no target). Drawn once,
                // in the first text node that contains it.
                if from < start || from > end || self.drawn.borrow().contains(&m.number) {
                    continue;
                }
                let at = if align.is_some() { to_text(from) } else { text.len() };
                cuts.push((at, Event::Point(m.number)));
            } else {
                if to <= start || from >= end {
                    continue;
                }
                let (a, b) = if align.is_some() {
                    (to_text(from.max(start)), to_text(to.min(end)))
                } else {
                    (0, text.len())
                };
                cuts.push((a, Event::Open(m.number)));
                // A mark across several nodes (say, into bold) gets its
                // marker and replacement once, where it ends.
                cuts.push((b, Event::Close(m.number, to <= end)));
            }
            self.drawn.borrow_mut().insert(m.number);
            self.pending_notes.borrow_mut().insert(m.number);
        }
        if cuts.is_empty() {
            return text.to_string();
        }
        // Closes before points before opens at the same offset.
        cuts.sort_by_key(|(at, e)| (*at, e.order()));

        let mut out = String::new();
        let mut pos = 0;
        for (at, event) in cuts {
            let at = floor_char_boundary(text, at);
            out.push_str(&text[pos.min(at)..at]);
            pos = pos.max(at);
            let mark = &self.marks[event.number() - 1];
            match event {
                Event::Open(_) => out.push_str(open_style(&mark.annotation.kind)),
                Event::Close(_, last) => {
                    out.push_str(&close_style(&mark.annotation.kind, fg));
                    if !last {
                        continue;
                    }
                    if let (AnnotationKind::Replace, Some(r)) = (&mark.annotation.kind, &mark.annotation.replacement) {
                        out.push_str(&format!("{GREEN}{r}{fg}"));
                    }
                    out.push_str(&marker(mark.number, fg));
                }
                Event::Point(_) => {
                    if let (AnnotationKind::Insert, Some(r)) = (&mark.annotation.kind, &mark.annotation.replacement) {
                        out.push_str(&format!("{GREEN}{r}{fg}"));
                    }
                    out.push_str(&marker(mark.number, fg));
                }
            }
        }
        out.push_str(&text[pos..]);
        out
    }

    /// Claim the marks inside a heading or table that its text didn't draw
    /// (one across two cells, say), so their notes follow it, not the end.
    pub fn claim_block(&self, start: usize, end: usize) {
        for m in &self.marks {
            let Some((from, to)) = m.range else { continue };
            if from >= start && to <= end && self.drawn.borrow_mut().insert(m.number) {
                self.pending_notes.borrow_mut().insert(m.number);
                self.in_block.borrow_mut().insert(m.number);
            }
        }
    }

    /// The notes of the marks drawn since the last call, for after a paragraph.
    pub fn take_notes(&self, indent: usize, width: usize) -> String {
        let numbers = std::mem::take(&mut *self.pending_notes.borrow_mut());
        let in_block = self.in_block.borrow();
        numbers.iter().map(|n| note(&self.marks[n - 1], in_block.contains(n), indent, width)).collect()
    }

    /// Annotations that were never drawn inline: they no longer anchor, or
    /// they sit in text that isn't rendered (code blocks, raw HTML).
    pub fn leftovers(&self, width: usize) -> String {
        let drawn = self.drawn.borrow();
        let rest: Vec<&Mark> = self.marks.iter().filter(|m| !drawn.contains(&m.number)).collect();
        if rest.is_empty() {
            return String::new();
        }
        let mut out = format!("\x1b[2m{}\x1b[0m\n", "─".repeat(width.min(40)));
        for m in rest {
            let why = if m.range.is_none() { "not found in document" } else { "not shown inline" };
            out.push_str(&format!("{PURPLE}[{}]\x1b[0m \x1b[2m{why}\x1b[0m\n", m.number));
            out.push_str(&note(m, true, 0, width));
        }
        out
    }
}

#[derive(Clone, Copy)]
enum Event {
    Open(usize),
    Close(usize, bool),
    Point(usize),
}

impl Event {
    fn number(self) -> usize {
        match self {
            Event::Open(n) | Event::Close(n, _) | Event::Point(n) => n,
        }
    }
    fn order(self) -> u8 {
        match self {
            Event::Close(..) => 0,
            Event::Point(_) => 1,
            Event::Open(_) => 2,
        }
    }
}

// Each style is switched off with its own code, never a full reset, so the
// bold or italic of the text around the annotation survives it.
fn open_style(kind: &AnnotationKind) -> &'static str {
    match kind {
        AnnotationKind::Replace | AnnotationKind::Delete => STRIKE_RED,
        AnnotationKind::Highlight | AnnotationKind::Comment => PURPLE_BG,
        AnnotationKind::Insert => "",
    }
}

fn close_style(kind: &AnnotationKind, fg: &str) -> String {
    match kind {
        AnnotationKind::Replace | AnnotationKind::Delete => format!("\x1b[29m{fg}"),
        AnnotationKind::Highlight | AnnotationKind::Comment => PURPLE_BG_OFF.to_string(),
        AnnotationKind::Insert => String::new(),
    }
}

fn marker(number: usize, fg: &str) -> String {
    format!("{PURPLE}[{number}]{fg}")
}

/// `[n] kind · source/author`, the change itself when it isn't drawn in the
/// text (`show_change`), the comment, then its replies, in a purple bar.
fn note(mark: &Mark, show_change: bool, indent: usize, width: usize) -> String {
    let a = &mark.annotation;
    let prefix = format!("{}{PURPLE}┃\x1b[0m ", " ".repeat(indent));
    let wrap_width = width.saturating_sub(indent + 2);
    let mut lines = vec![format!(
        "{PURPLE}[{}]\x1b[0m \x1b[1m{}\x1b[0m \x1b[2m· {}/{} · {}\x1b[0m",
        mark.number, a.kind, a.source, a.author, a.id
    )];
    if show_change {
        lines.push(change(a));
    }
    if let Some(c) = &a.comment {
        lines.push(markdown(c, wrap_width));
    }
    for r in &a.replies {
        lines.push(format!("\x1b[2m↳ {}/{}:\x1b[0m {}", r.source, r.author, markdown(&r.body, wrap_width)));
    }
    let mut out = String::new();
    for line in lines.join("\n").lines() {
        out.push_str(&format!("{prefix}{line}\n"));
    }
    out.push('\n');
    out
}

/// What the annotation does, styled as it would be in the text: “old” → “new”.
fn change(a: &Annotation) -> String {
    let target = a.target.as_deref().unwrap_or("");
    let replacement = a.replacement.as_deref().unwrap_or("");
    match a.kind {
        AnnotationKind::Replace => format!("{STRIKE_RED}{target}{STRIKE_RED_OFF} → {GREEN}{replacement}{GREEN_OFF}"),
        AnnotationKind::Delete => format!("{STRIKE_RED}{target}{STRIKE_RED_OFF}"),
        AnnotationKind::Insert => format!("\x1b[2mafter “{}”\x1b[0m {GREEN}+ {replacement}{GREEN_OFF}", a.context_before),
        AnnotationKind::Highlight | AnnotationKind::Comment => format!("{PURPLE_BG}{target}{PURPLE_BG_OFF}"),
    }
}

/// A comment or reply body, which is markdown, rendered to `width` columns.
/// Falls back to the plain text if it can't be rendered.
fn markdown(text: &str, width: usize) -> String {
    let config = Config {
        width: width.max(20) as u16,
        images: false,
        mermaid: false,
        mermaid_binary: String::new(),
        theme: "base16-ocean.dark".into(),
        image_protocol: None,
        kitty_store: None,
        annotations: false,
        overlay: None,
    };
    match render_document(text, None, &config) {
        Ok(out) => out.trim().to_string(),
        Err(_) => text.trim().to_string(),
    }
}

/// `(text byte, source byte)` at each char of `text` and at its end, walking
/// `text` and the markdown it was parsed from in step: entities (`&amp;`) and
/// backslash escapes are where the two differ.
fn align(text: &str, source: &str) -> Option<Vec<(usize, usize)>> {
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut src = 0;
    for (t, ch) in text.char_indices() {
        let rest = &source[src..];
        let width = if let Some(n) = entity_len(rest) {
            n
        } else if rest.starts_with(ch) {
            ch.len_utf8()
        } else if rest.starts_with('\\') && rest[1..].starts_with(ch) {
            1 + ch.len_utf8()
        } else {
            return None;
        };
        map.push((t, src));
        src += width;
    }
    map.push((text.len(), src));
    Some(map)
}

/// The length of the entity (`&amp;`, `&#38;`) `s` starts with, if any.
fn entity_len(s: &str) -> Option<usize> {
    let body = s.strip_prefix('&')?;
    let n = body.find(';').filter(|&n| n > 0 && n <= 32)?;
    body[..n].chars().all(|c| c.is_ascii_alphanumeric() || c == '#').then_some(n + 2)
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use folio::sidecar::AnnotationSource;
    use std::rc::Rc;

    fn ann(kind: AnnotationKind, context_before: &str, target: Option<&str>, replacement: Option<&str>) -> Annotation {
        Annotation {
            id: format!("ann_{context_before}"),
            kind,
            source: AnnotationSource::Agent,
            author: "claude".into(),
            context_before: context_before.into(),
            target: target.map(Into::into),
            replacement: replacement.map(Into::into),
            comment: Some("why".into()),
            created: String::new(),
            resolved: false,
            resolved_as: None,
            resolved_at: None,
            replies: Vec::new(),
        }
    }

    fn render(md: &str, annotations: Vec<Annotation>) -> String {
        let config = Config {
            width: 200,
            images: false,
            mermaid: false,
            mermaid_binary: String::new(),
            theme: String::new(),
            image_protocol: None,
            kitty_store: None,
            annotations: true,
            overlay: Overlay::new(md, annotations).map(Rc::new),
        };
        render_document(md, None, &config).unwrap()
    }

    #[test]
    fn comments_and_replies_are_rendered_as_markdown() {
        let mut a = ann(AnnotationKind::Comment, "Body", None, None);
        a.comment = Some("Use **bold** and `code`.\n\n- one\n- two".into());
        a.replies.push(folio::sidecar::ThreadReply {
            id: "r1".into(),
            author: "me".into(),
            source: AnnotationSource::Local,
            body: "*thanks*".into(),
            created: String::new(),
        });
        let out = render("Body.\n", vec![a]);
        assert!(!out.contains("**bold**") && !out.contains("`code`") && !out.contains("*thanks*"), "{out:?}");
        assert!(out.contains("\x1b[1mbold"), "{out:?}");
        assert!(out.contains("one") && out.contains("two"), "{out:?}");
        let list = out.lines().filter(|l| l.contains("one") || l.contains("two")).count();
        assert_eq!(list, 2, "{out:?}");
    }

    #[test]
    fn align_steps_over_entities_and_escapes() {
        let map = align("a & b*", "a &amp; b\\*").unwrap();
        assert_eq!(map, vec![(0, 0), (1, 1), (2, 2), (3, 7), (4, 8), (5, 9), (6, 11)]);
    }

    #[test]
    fn align_gives_up_when_text_and_source_differ() {
        assert!(align("abc", "xyz").is_none());
    }

    #[test]
    fn replace_strikes_the_target_and_shows_the_replacement() {
        let out = render("Ship it today.\n", vec![ann(AnnotationKind::Replace, "Ship it ", Some("today"), Some("tomorrow"))]);
        assert!(out.contains(&format!("Ship it {STRIKE_RED}today{STRIKE_RED_OFF}{GREEN}tomorrow{GREEN_OFF}{}", marker(1, DEFAULT_FG))), "{out:?}");
        assert!(out.contains("[1]\x1b[0m \x1b[1mreplace"), "note follows the paragraph: {out:?}");
    }

    #[test]
    fn a_mark_into_bold_keeps_the_bold_and_is_numbered_once() {
        let out = render("A **bold** word.\n", vec![ann(AnnotationKind::Delete, "A ", Some("bold word"), None)]);
        assert!(out.contains(&format!("\x1b[1m{STRIKE_RED}bold{STRIKE_RED_OFF}\x1b[0m{STRIKE_RED} word{STRIKE_RED_OFF}{}", marker(1, DEFAULT_FG))), "{out:?}");
        assert_eq!(out.matches(&marker(1, DEFAULT_FG)).count(), 1, "{out:?}");
    }

    #[test]
    fn a_mark_after_an_entity_lands_on_its_words() {
        let out = render("Fish &amp; chips here.\n", vec![ann(AnnotationKind::Highlight, "Fish & ", Some("chips"), None)]);
        assert!(out.contains(&format!("Fish & {PURPLE_BG}chips{PURPLE_BG_OFF}")), "{out:?}");
    }

    #[test]
    fn an_insert_is_drawn_once_at_its_point() {
        let out = render("Back up the tables first.\n", vec![ann(AnnotationKind::Insert, "the tables", None, Some(" and logs"))]);
        assert!(out.contains(&format!("the tables{GREEN} and logs{GREEN_OFF}{} first.", marker(1, DEFAULT_FG))), "{out:?}");
        assert_eq!(out.matches(" and logs").count(), 1, "{out:?}");
    }

    #[test]
    fn unanchored_annotations_are_listed_at_the_end() {
        let out = render("Body.\n", vec![ann(AnnotationKind::Delete, "Gone ", Some("text"), None)]);
        assert!(out.contains("not found in document"), "{out:?}");
        assert!(out.contains(&format!("{STRIKE_RED}text{STRIKE_RED_OFF}")), "{out:?}");
    }

    #[test]
    fn a_heading_annotation_is_drawn_in_it_and_noted_under_it() {
        let out = render("## Old title\n\nBody.\n", vec![ann(AnnotationKind::Replace, "", Some("Old"), Some("New"))]);
        // The heading's cyan comes back after the replacement and the marker.
        let cyan = "\x1b[36m";
        assert!(out.contains(&format!("{STRIKE_RED}Old\x1b[29m{cyan}{GREEN}New{cyan}{} title", marker(1, cyan))), "{out:?}");
        // "OldNew[1] title" is 15 columns; a level-2 underline is 2 longer.
        assert!(out.contains(&"─".repeat(17)) && !out.contains(&"─".repeat(18)), "underline fits the heading: {out:?}");
        let note = out.find("[1]\x1b[0m \x1b[1mreplace").expect("note");
        assert!(note < out.find("Body.").unwrap(), "{out:?}");
        assert!(!out.contains(" → "), "drawn in the heading, so not repeated in the note: {out:?}");
    }

    #[test]
    fn a_table_annotation_is_noted_under_the_table() {
        let md = "| Step | Owner |\n|---|---|\n| Migrate | Ann |\n\nAfter.\n";
        let out = render(md, vec![ann(AnnotationKind::Replace, "StepOwnerMigrate", Some("Ann"), Some("Bo"))]);
        assert!(out.contains(&format!("{STRIKE_RED}Ann{STRIKE_RED_OFF}{GREEN}Bo{GREEN_OFF}{}", marker(1, DEFAULT_FG))), "drawn in the cell: {out:?}");
        let note = out.find("[1]\x1b[0m \x1b[1mreplace").expect("note");
        assert!(out.find('└').unwrap() < note && note < out.find("After.").unwrap(), "{out:?}");
        // The borders still line up around the wider cell.
        let rows: Vec<usize> = out.lines().filter(|l| l.contains('│') || l.contains('┌')).map(crate::render::markdown::visible_len).collect();
        assert!(rows.windows(2).all(|w| w[0] == w[1]), "{rows:?} in {out:?}");
    }

    #[test]
    fn resolved_annotations_are_not_shown() {
        let mut a = ann(AnnotationKind::Delete, "Ship it ", Some("today"), None);
        a.resolved = true;
        assert!(Overlay::new("Ship it today.\n", vec![a]).is_none());
    }
}
