use anyhow::Result;
use comrak::{Arena, Options, parse_document};
use comrak::nodes::{AstNode, NodeValue, NodeHeading, ListType, TableAlignment};
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use crate::render::Config;
use crate::render::code::highlight_code;
use crate::render::images::render_image;
use crate::render::mermaid::render_mermaid;
use crate::render::links::osc8_link_for;

/// Parse and render a markdown document to an ANSI string.
pub fn render_document(markdown: &str, base_dir: Option<&Path>, config: &Config) -> Result<String> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());

    let root = parse_document(&arena, markdown, &options);

    let mut output = String::new();
    render_node(root, &mut output, base_dir, config, 0, false)?;
    Ok(output)
}

fn render_node<'a>(
    node: &'a AstNode<'a>,
    output: &mut String,
    base_dir: Option<&Path>,
    config: &Config,
    indent: usize,
    in_tight_list: bool,
) -> Result<()> {
    let data = node.data.borrow();

    match &data.value {
        NodeValue::Document => {
            drop(data);
            for child in node.children() {
                render_node(child, output, base_dir, config, indent, false)?;
            }
        }

        NodeValue::Heading(NodeHeading { level, .. }) => {
            let level = *level;
            drop(data);

            let mut text = String::new();
            collect_text(node, &mut text);
            let width = config.width as usize;

            match level {
                1 => {
                    let line = "═".repeat(width.min(display_width(&text) + 4));
                    output.push_str(&format!("\n\x1b[1;34m{line}\x1b[0m\n"));
                    output.push_str(&format!("\x1b[1;34m  {text}\x1b[0m\n"));
                    output.push_str(&format!("\x1b[1;34m{line}\x1b[0m\n\n"));
                }
                2 => {
                    let line = "─".repeat(width.min(display_width(&text) + 2));
                    output.push_str(&format!("\n\x1b[1;36m{text}\x1b[0m\n"));
                    output.push_str(&format!("\x1b[36m{line}\x1b[0m\n\n"));
                }
                3 => output.push_str(&format!("\n\x1b[1;33m### {text}\x1b[0m\n\n")),
                4 => output.push_str(&format!("\n\x1b[1m#### {text}\x1b[0m\n\n")),
                _ => output.push_str(&format!("\n\x1b[1m{text}\x1b[0m\n\n")),
            }
        }

        NodeValue::Paragraph => {
            drop(data);
            let mut inline_out = String::new();
            for child in node.children() {
                render_inline(child, &mut inline_out, base_dir, config)?;
            }
            let prefix = " ".repeat(indent);
            let wrapped = word_wrap(&inline_out, config.width as usize, indent);
            output.push_str(&format!("{prefix}{wrapped}\n"));
            if !in_tight_list {
                output.push('\n');
            }
        }

        NodeValue::BlockQuote => {
            drop(data);
            let mut inner = String::new();
            // The indent and the "│ " prefix take columns; wrap the contents narrower to compensate.
            let mut inner_config = config.clone();
            inner_config.width = config.width.saturating_sub(indent as u16 + 2);
            let pad = " ".repeat(indent);
            for child in node.children() {
                render_node(child, &mut inner, base_dir, &inner_config, 0, false)?;
            }
            // Block children end with a blank separator line; don't draw it as a quote line.
            for line in inner.trim_end_matches('\n').lines() {
                // Inline resets (\x1b[0m) would end the quote's italics; re-apply after each.
                let line = line
                    .replace("\x1b[0m", "\x1b[0m\x1b[3m")
                    .replace("\x1b[23m", "\x1b[23m\x1b[3m");
                output.push_str(&format!("{pad}\x1b[2m│\x1b[0m \x1b[3m{line}\x1b[0m\n"));
            }
            output.push('\n');
        }

        NodeValue::CodeBlock(cb) => {
            let lang = cb.info.split_whitespace().next().unwrap_or("").to_string();
            let code = cb.literal.clone();
            drop(data);

            if lang == "mermaid" && config.mermaid {
                match render_mermaid(&code, config) {
                    Ok(img_str) => output.push_str(&img_str),
                    Err(e) => {
                        eprintln!("mdcat: mermaid render failed: {e}");
                        output.push_str(&render_code_block(&code, "", config, indent));
                        output.push_str("\x1b[2m[Install mmdc to render Mermaid diagrams: npm install -g @mermaid-js/mermaid-cli]\x1b[0m\n\n");
                    }
                }
            } else {
                output.push_str(&render_code_block(&code, &lang, config, indent));
            }
        }

        NodeValue::HtmlBlock(_) => {
            // Skip raw HTML blocks
            drop(data);
        }

        NodeValue::List(list) => {
            let list_type = list.list_type;
            let tight = list.tight;
            let start = list.start;
            drop(data);

            let mut counter = start;
            for child in node.children() {
                let child_data = child.data.borrow();
                // Task items still occupy a position in an ordered list.
                let number = counter;
                counter += 1;
                let bullet = match &child_data.value {
                    NodeValue::TaskItem(checked) => {
                        if checked.symbol.is_some() { "☑".to_string() } else { "☐".to_string() }
                    }
                    _ => match list_type {
                        ListType::Bullet => "•".to_string(),
                        ListType::Ordered => format!("{number}."),
                    },
                };
                drop(child_data);
                render_list_item(child, output, base_dir, config, indent, tight, &bullet)?;
            }
            if !in_tight_list {
                output.push('\n');
            }
        }

        NodeValue::Item(_) => {
            // Items are rendered by their parent List via render_list_item
            drop(data);
        }

        NodeValue::ThematicBreak => {
            drop(data);
            // Stay inside the enclosing list item's indent so the rule fits the terminal.
            let line = "─".repeat((config.width as usize).saturating_sub(indent));
            let prefix = " ".repeat(indent);
            output.push_str(&format!("{prefix}\x1b[2m{line}\x1b[0m\n\n"));
        }

        NodeValue::Table(table) => {
            let alignments = table.alignments.clone();
            drop(data);
            // Render at the width left over after the indent, then indent each line.
            let inner = Config { width: config.width.saturating_sub(indent as u16).max(1), ..config.clone() };
            let mut table_out = String::new();
            render_table(node, &mut table_out, &inner, &alignments)?;
            let pad = " ".repeat(indent);
            for line in table_out.split_inclusive('\n') {
                if line == "\n" {
                    output.push_str(line);
                } else {
                    output.push_str(&pad);
                    output.push_str(line);
                }
            }
        }

        NodeValue::FootnoteDefinition(def) => {
            let label = format!("[{}]:", def.name);
            drop(data);
            render_list_item(node, output, base_dir, config, indent, false, &label)?;
        }

        NodeValue::FrontMatter(content) => {
            let content = content.clone();
            drop(data);
            let width = config.width as usize;

            // comrak includes the --- delimiters in the content; strip them
            let body: Vec<&str> = content
                .lines()
                .filter(|l| l.trim() != "---")
                .collect();

            let sep = "─".repeat(width.min(40));
            output.push_str(&format!("\x1b[2m{sep}\x1b[0m\n"));
            for line in body {
                if line.is_empty() {
                    output.push('\n');
                } else {
                    let wrapped = word_wrap(line, width, 2);
                    output.push_str(&format!("\x1b[2m{wrapped}\x1b[0m\n"));
                }
            }
            output.push_str(&format!("\x1b[2m{sep}\x1b[0m\n\n"));
        }

        _ => {
            drop(data);
            // Fallback: render children
            for child in node.children() {
                render_node(child, output, base_dir, config, indent, in_tight_list)?;
            }
        }
    }

    Ok(())
}

fn render_inline<'a>(
    node: &'a AstNode<'a>,
    output: &mut String,
    base_dir: Option<&Path>,
    config: &Config,
) -> Result<()> {
    let data = node.data.borrow();

    match &data.value {
        NodeValue::Text(text) => {
            output.push_str(text);
            drop(data);
        }

        NodeValue::SoftBreak => {
            output.push(' ');
            drop(data);
        }

        NodeValue::FootnoteReference(reference) => {
            let name = reference.name.clone();
            drop(data);
            output.push_str(&format!("\x1b[2m[{name}]\x1b[0m"));
        }

        NodeValue::LineBreak => {
            output.push('\n');
            drop(data);
        }

        NodeValue::Strong => {
            drop(data);
            output.push_str("\x1b[1m");
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
            }
            // Switch off only bold so enclosing styles stay active.
            output.push_str("\x1b[22m");
        }

        NodeValue::Emph => {
            drop(data);
            output.push_str("\x1b[3m");
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
            }
            output.push_str("\x1b[23m");
        }

        NodeValue::Strikethrough => {
            drop(data);
            output.push_str("\x1b[9m");
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
            }
            output.push_str("\x1b[29m");
        }

        NodeValue::Code(code) => {
            let text = code.literal.clone();
            drop(data);
            output.push_str(&format!("\x1b[48;5;236m\x1b[96m {text} \x1b[49m\x1b[39m"));
        }

        NodeValue::Link(link) => {
            let url = link.url.clone();
            drop(data);
            let mut label = String::new();
            for child in node.children() {
                render_inline(child, &mut label, base_dir, config)?;
            }
            let mut plain = String::new();
            collect_text(node, &mut plain);
            output.push_str(&osc8_link_for(&url, &label, &plain));
        }

        NodeValue::Image(img) => {
            let url = img.url.clone();
            drop(data);
            let mut alt = String::new();
            collect_text(node, &mut alt);

            if config.images {
                match render_image(&url, &alt, base_dir, config) {
                    Ok(img_str) => output.push_str(&img_str),
                    Err(e) => {
                        eprintln!("mdcat: image render failed ({url}): {e}");
                        output.push_str(&format!("\x1b[2m[image: {alt}]\x1b[0m"));
                    }
                }
            } else {
                output.push_str(&format!("\x1b[2m[image: {alt}]\x1b[0m"));
            }
        }

        _ => {
            drop(data);
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
            }
        }
    }

    Ok(())
}

fn render_list_item<'a>(
    node: &'a AstNode<'a>,
    output: &mut String,
    base_dir: Option<&Path>,
    config: &Config,
    parent_indent: usize,
    tight: bool,
    bullet: &str,
) -> Result<()> {
    let indent = parent_indent + 2;
    let prefix = " ".repeat(parent_indent);
    output.push_str(&format!("{prefix}\x1b[1m{bullet}\x1b[0m "));

    for (i, child) in node.children().enumerate() {
        let mut child_out = String::new();
        render_node(child, &mut child_out, base_dir, config, indent, tight)?;
        // The bullet already occupies the first child's indent; don't pad it twice.
        if i == 0 {
            if let Some(rest) = child_out.strip_prefix(&" ".repeat(indent)) {
                output.push_str(rest);
                continue;
            }
        }
        output.push_str(&child_out);
    }

    // An empty item has no children, so nothing ended the bullet's line.
    if !output.ends_with('\n') {
        output.push('\n');
    }

    Ok(())
}

fn render_code_block(code: &str, lang: &str, config: &Config, indent: usize) -> String {
    // The trailing newline would otherwise render as an extra empty row.
    let highlighted = highlight_code(code.trim_end_matches('\n'), lang, &config.theme);
    let width = (config.width as usize).saturating_sub(indent);
    let inner = width.min(60);
    let line = "─".repeat(inner);
    let lang_label = if lang.is_empty() { String::new() } else { format!(" {lang} ") };
    // The label replaces part of the top border so both borders are equally wide.
    let top_line = "─".repeat(inner.saturating_sub(lang_label.chars().count()));

    let pad = " ".repeat(indent);
    let mut out = String::new();
    out.push_str(&format!("{pad}\x1b[2m┌{lang_label}{top_line}┐\x1b[0m\n"));
    for line_str in highlighted.lines() {
        out.push_str(&format!("{pad}\x1b[2m│\x1b[0m {line_str}\n"));
    }
    out.push_str(&format!("{pad}\x1b[2m└{line}┘\x1b[0m\n\n"));
    out
}

fn render_table<'a>(
    node: &'a AstNode<'a>,
    output: &mut String,
    config: &Config,
    alignments: &[TableAlignment],
) -> Result<()> {
    // Collect rows
    let mut rows: Vec<Vec<String>> = Vec::new();

    for row in node.children() {
        let mut cells: Vec<String> = Vec::new();
        for cell in row.children() {
            let mut cell_text = String::new();
            collect_text(cell, &mut cell_text);
            cells.push(cell_text);
        }
        rows.push(cells);
    }

    if rows.is_empty() {
        return Ok(());
    }

    // Calculate column widths
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths: Vec<usize> = vec![0; cols];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            col_widths[i] = col_widths[i].max(display_width(cell));
        }
    }

    // Total width: left border + (col_width + 2 padding) per col + separator per col + right border
    let total_width: usize = 1 + col_widths.iter().map(|w| w + 2).sum::<usize>() + cols;

    if total_width > config.width as usize {
        render_stacked_table(&rows, output, config.width as usize);
        return Ok(());
    }

    let sep: String = col_widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┼");
    let top: String = col_widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┬");
    let bot: String = col_widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┴");

    output.push_str(&format!("\x1b[2m┌{top}┐\x1b[0m\n"));

    for (i, row) in rows.iter().enumerate() {
        let cells: String = row.iter().enumerate().map(|(j, cell)| {
            let width = col_widths.get(j).copied().unwrap_or(0);
            let align = alignments.get(j).copied().unwrap_or(TableAlignment::None);
            if i == 0 {
                format!(" \x1b[1m{}\x1b[0m ", pad_aligned(cell, width, align))
            } else {
                format!(" {} ", pad_aligned(cell, width, align))
            }
        }).collect::<Vec<_>>().join("\x1b[2m│\x1b[0m");

        output.push_str(&format!("\x1b[2m│\x1b[0m{cells}\x1b[2m│\x1b[0m\n"));

        if i == 0 {
            output.push_str(&format!("\x1b[2m├{sep}┤\x1b[0m\n"));
        }
    }

    output.push_str(&format!("\x1b[2m└{bot}┘\x1b[0m\n\n"));
    Ok(())
}

fn render_stacked_table(rows: &[Vec<String>], output: &mut String, width: usize) {
    let headers = &rows[0];
    let label_width = headers.iter().map(|h| display_width(h)).max().unwrap_or(0);

    // No body rows to stack: show the headers so the table isn't silently dropped.
    if rows.len() == 1 {
        for header in headers {
            output.push_str(&format!("\x1b[1m{header}\x1b[0m\n"));
        }
        output.push('\n');
        return;
    }

    for (i, row) in rows[1..].iter().enumerate() {
        let row_label = format!(" Row {} ", i + 1);
        let dash_total = width.saturating_sub(row_label.chars().count()).max(4);
        let left = dash_total / 2;
        let right = dash_total - left;
        output.push_str(&format!(
            "\x1b[2m{}{}{}\x1b[0m\n",
            "─".repeat(left),
            row_label,
            "─".repeat(right)
        ));

        for (j, header) in headers.iter().enumerate() {
            let value = row.get(j).map(|s| s.as_str()).unwrap_or("");
            output.push_str(&format!("\x1b[1m{}\x1b[0m : {}\n", pad_to(header, label_width), value));
        }
        output.push('\n');
    }
}

/// Collect all plain text from a node and its children.
fn collect_text<'a>(node: &'a AstNode<'a>, output: &mut String) {
    let data = node.data.borrow();
    match &data.value {
        NodeValue::Text(text) => output.push_str(text),
        NodeValue::Code(code) => output.push_str(&code.literal),
        NodeValue::SoftBreak | NodeValue::LineBreak => output.push(' '),
        _ => {
            drop(data);
            for child in node.children() {
                collect_text(child, output);
            }
        }
    }
}

/// Terminal display width of plain text (wide CJK characters take two columns).
fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Pad `s` with spaces on the right up to `width` display columns.
fn pad_to(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(display_width(s))))
}

/// Pad `s` to `width` display columns according to a table column alignment.
fn pad_aligned(s: &str, width: usize, align: TableAlignment) -> String {
    let gap = width.saturating_sub(display_width(s));
    match align {
        TableAlignment::Right => format!("{}{s}", " ".repeat(gap)),
        TableAlignment::Center => {
            let left = gap / 2;
            format!("{}{s}{}", " ".repeat(left), " ".repeat(gap - left))
        }
        _ => pad_to(s, width),
    }
}

/// Word-wrap text at the given width, accounting for current indent.
fn word_wrap(text: &str, width: usize, indent: usize) -> String {
    // Strip ANSI for width calculation would be complex; use a simple approach
    // that wraps on whitespace boundaries. ANSI codes don't add visual width.
    let available = width.saturating_sub(indent).max(20);
    let mut result = String::new();
    let mut line_len = 0usize;

    // Explicit newlines (hard line breaks) are preserved; each segment wraps independently.
    for (i, segment) in text.split('\n').enumerate() {
        if i > 0 {
            result.push('\n');
            result.push_str(&" ".repeat(indent));
            line_len = 0;
        }
        for word in split_words(segment) {
            // Approximate visible length (ignores ANSI codes)
            let word_len = visible_len(word);
            if line_len == 0 {
                result.push_str(word);
                line_len = word_len;
            } else if line_len + 1 + word_len > available {
                result.push('\n');
                result.push_str(&" ".repeat(indent));
                result.push_str(word);
                line_len = word_len;
            } else {
                result.push(' ');
                result.push_str(word);
                line_len += 1 + word_len;
            }
        }
    }

    result
}

const CODE_START: &str = "\x1b[48;5;236m";
const CODE_END: &str = "\x1b[49m\x1b[39m";

/// Split on whitespace, but keep an inline code span intact so runs of spaces
/// inside it are not collapsed to a single space.
fn split_words(segment: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut rest = segment;
    let mut start: Option<usize> = None; // byte offset of the current word in `segment`
    let base = segment.as_ptr() as usize;
    while !rest.is_empty() {
        let pos = rest.as_ptr() as usize - base;
        if rest.starts_with(CODE_START) {
            start.get_or_insert(pos);
            let end = rest.find(CODE_END).map_or(rest.len(), |i| i + CODE_END.len());
            rest = &rest[end..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        if ch.is_whitespace() {
            if let Some(s) = start.take() {
                words.push(&segment[s..pos]);
            }
        } else {
            start.get_or_insert(pos);
        }
        rest = &rest[ch.len_utf8()..];
    }
    if let Some(s) = start {
        words.push(&segment[s..]);
    }
    words
}

/// Estimate visible length of a string, ignoring ANSI escape sequences.
///
/// Handles CSI sequences (`ESC [ ... final-byte`) and OSC sequences such as
/// OSC 8 hyperlinks (`ESC ] ... BEL` or `ESC ] ... ESC \`).
pub(crate) fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            len += UnicodeWidthChar::width(ch).unwrap_or(0);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Config;

    fn test_config(width: u16) -> Config {
        Config {
            width,
            images: false,
            mermaid: false,
            mermaid_binary: String::new(),
            theme: String::new(),
            image_protocol: None,
            kitty_store: None,
        }
    }

    fn code_block_borders(result: &str) -> (usize, usize) {
        let top = result.lines().find(|l| l.contains('┌')).expect("top border");
        let bottom = result.lines().find(|l| l.contains('└')).expect("bottom border");
        (visible_len(top), visible_len(bottom))
    }

    #[test]
    fn code_block_borders_match_with_language_label() {
        let result = render_code_block("fn main() {}\n", "rust", &test_config(80), 0);
        let (top, bottom) = code_block_borders(&result);
        assert_eq!(top, bottom, "got: {result:?}");
    }

    #[test]
    fn code_block_borders_match_without_language_label() {
        let result = render_code_block("plain\n", "", &test_config(80), 0);
        let (top, bottom) = code_block_borders(&result);
        assert_eq!(top, bottom, "got: {result:?}");
    }

    #[test]
    fn code_block_has_no_trailing_empty_row() {
        let result = render_code_block("one\ntwo\n", "", &test_config(80), 0);
        let rows = result.lines().filter(|l| l.contains('│')).count();
        assert_eq!(rows, 2, "got: {result:?}");
    }

    #[test]
    fn inline_code_keeps_repeated_spaces() {
        let result = render_document("a `x  y` b", None, &test_config(80)).unwrap();
        assert!(result.contains(" x  y "), "got: {result:?}");
    }

    #[test]
    fn split_words_splits_plain_text_and_keeps_code_spans() {
        assert_eq!(split_words("a  b\tc"), vec!["a", "b", "c"]);
        assert_eq!(split_words(""), Vec::<&str>::new());
        let code = format!("{CODE_START} p  q {CODE_END}");
        let text = format!("go {code}, now");
        assert_eq!(split_words(&text), vec!["go", &format!("{code},"), "now"]);
    }

    #[test]
    fn footnote_reference_is_rendered() {
        let md = "Hello[^1]\n\n[^1]: The note";
        let result = render_document(md, None, &test_config(80)).unwrap();
        assert!(result.contains("Hello\x1b[2m[1]\x1b[0m"), "got: {result:?}");
    }

    #[test]
    fn footnote_definition_is_rendered() {
        let md = "Hello[^note]\n\n[^note]: The note text";
        let result = render_document(md, None, &test_config(80)).unwrap();
        assert!(result.contains("[note]:"), "got: {result:?}");
        assert!(result.contains("The note text"), "got: {result:?}");
    }

    #[test]
    fn frontmatter_wraps_long_lines() {
        let md = "---\ntitle: This is a very long title that should definitely wrap at the given terminal width\n---\n\nBody text.";
        let result = render_document(md, None, &test_config(40)).unwrap();
        for line in result.lines() {
            assert!(
                visible_len(line) <= 40,
                "line exceeds width: {line:?}"
            );
        }
    }

    #[test]
    fn frontmatter_short_lines_pass_through() {
        let md = "---\ntitle: Short\ndate: 2024-01-01\n---\n\nBody.";
        let result = render_document(md, None, &test_config(80)).unwrap();
        assert!(result.contains("title: Short"));
        assert!(result.contains("date: 2024-01-01"));
    }

    #[test]
    fn visible_len_counts_wide_characters_as_two_columns() {
        assert_eq!(visible_len("日本語"), 6);
        assert_eq!(visible_len("\x1b[1m日本\x1b[0m"), 4);
    }

    #[test]
    fn table_columns_align_with_wide_characters() {
        let md = "| 名前 | b |\n|---|---|\n| ab | c |\n";
        let result = render_document(md, None, &test_config(80)).unwrap();
        let widths: Vec<usize> = result
            .lines()
            .filter(|l| l.contains('│') || l.contains('┌') || l.contains('└'))
            .map(visible_len)
            .collect();
        assert!(widths.len() >= 3);
        assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}");
    }

    #[test]
    fn table_honours_column_alignment() {
        let md = "| left | center | right |\n|:--|:-:|--:|\n| a | b | c |\n";
        let result = render_document(md, None, &test_config(80)).unwrap();
        let row = result.lines().map(|l| l.replace("\x1b[2m", "").replace("\x1b[0m", "")).find(|l| l.contains(" a ")).unwrap();
        assert_eq!(row, "│ a    │   b    │     c │");
    }

    #[test]
    fn pad_aligned_variants() {
        assert_eq!(pad_aligned("ab", 5, TableAlignment::None), "ab   ");
        assert_eq!(pad_aligned("ab", 5, TableAlignment::Left), "ab   ");
        assert_eq!(pad_aligned("ab", 5, TableAlignment::Right), "   ab");
        assert_eq!(pad_aligned("ab", 5, TableAlignment::Center), " ab  ");
    }

    #[test]
    fn word_wrap_respects_wide_character_width() {
        let text = "日本語日本語日本語日本語日本語 日本語日本語日本語日本語日本語";
        let wrapped = word_wrap(text, 40, 0);
        assert!(wrapped.lines().all(|l| visible_len(l) <= 40), "{wrapped:?}");
        assert_eq!(wrapped.lines().count(), 2);
    }

    #[test]
    fn table_width_counts_chars_not_bytes() {
        // Table is 10 columns wide (6 chars + padding + borders): fits at width 10.
        let md = "| ééééé |\n|---|\n| ééééé |\n";
        let result = render_document(md, None, &test_config(10)).unwrap();
        assert!(result.contains('┌'), "table should not fall back to stacked layout: {result:?}");
    }

    #[test]
    fn stacked_table_aligns_non_ascii_headers() {
        let rows = vec![
            vec!["Nämé".to_string(), "x".to_string()],
            vec!["a".to_string(), "b".to_string()],
        ];
        let mut out = String::new();
        render_stacked_table(&rows, &mut out, 40);
        assert!(out.contains("Nämé\x1b[0m : a"), "{out:?}");
        assert!(out.contains("x   \x1b[0m : b"), "{out:?}");
    }

    #[test]
    fn ordered_list_honours_start_number() {
        let result = render_document("3. three\n4. four\n", None, &test_config(80)).unwrap();
        assert!(result.contains("3.\x1b[0m"), "{result:?}");
        assert!(result.contains("4.\x1b[0m"), "{result:?}");
        assert!(!result.contains("1."), "{result:?}");
    }

    #[test]
    fn ordered_list_numbering_continues_after_task_item() {
        let result = render_document("1. [x] first\n2. second\n3. third\n", None, &test_config(80)).unwrap();
        assert!(result.contains("2.\x1b[0m second"), "{result:?}");
        assert!(result.contains("3.\x1b[0m third"), "{result:?}");
    }

    #[test]
    fn ordered_list_default_starts_at_one() {
        let result = render_document("1. a\n2. b\n", None, &test_config(80)).unwrap();
        assert!(result.contains("1.\x1b[0m") && result.contains("2.\x1b[0m"), "{result:?}");
    }

    #[test]
    fn heading_underline_counts_chars_not_bytes() {
        let result = render_document("# ééééé\n\n## ééééé\n", None, &test_config(80)).unwrap();
        assert!(result.contains(&"═".repeat(9)) && !result.contains(&"═".repeat(10)), "{result:?}");
        assert!(result.contains(&"─".repeat(7)) && !result.contains(&"─".repeat(8)), "{result:?}");
    }

    #[test]
    fn hard_line_break_is_preserved() {
        let result = render_document("first  \nsecond\n", None, &test_config(80)).unwrap();
        assert!(result.contains("first\nsecond"), "{result:?}");
    }

    #[test]
    fn hard_line_break_is_indented_in_list_item() {
        let result = render_document("- first  \n  second\n", None, &test_config(80)).unwrap();
        assert!(result.contains("first\n  second"), "{result:?}");
    }

    #[test]
    fn word_wrap_keeps_newlines_and_resets_line_length() {
        assert_eq!(word_wrap("a b\nc d", 80, 0), "a b\nc d");
        // Second line starts fresh, so "cc dd" fits even though the total would not.
        assert_eq!(word_wrap("aaaaaaaaaaaaaaaaaaaa\ncc dd", 25, 0), "aaaaaaaaaaaaaaaaaaaa\ncc dd");
    }

    #[test]
    fn visible_len_ignores_osc8_hyperlink_sequences() {
        let link = crate::render::links::osc8_link("https://example.com/a", "label");
        // "label" + " (→ https://example.com/a)"
        assert_eq!(visible_len(&link), "label (→ https://example.com/a)".chars().count());
    }

    #[test]
    fn autolink_does_not_repeat_url_but_named_link_does() {
        let auto = render_document("<https://x.com>\n", None, &test_config(80)).unwrap();
        assert!(!auto.contains('→'), "{auto:?}");
        let mail = render_document("<me@x.com>\n", None, &test_config(80)).unwrap();
        assert!(!mail.contains('→'), "{mail:?}");
        let named = render_document("[site](https://x.com)\n", None, &test_config(80)).unwrap();
        assert!(named.contains("(→ https://x.com)"), "{named:?}");
    }

    #[test]
    fn list_item_text_follows_bullet_with_single_space() {
        let result = render_document("- a\n- b\n", None, &test_config(80)).unwrap();
        assert!(result.contains("•\x1b[0m a\n"), "{result:?}");
        assert!(result.contains("•\x1b[0m b\n"), "{result:?}");
    }

    #[test]
    fn empty_list_item_ends_its_line() {
        let result = render_document("- a\n-\n- c\n", None, &test_config(80)).unwrap();
        assert!(result.contains("•\x1b[0m a\n"), "{result:?}");
        assert!(result.contains("•\x1b[0m \n"), "{result:?}");
        assert!(result.contains("•\x1b[0m c\n"), "{result:?}");
        assert_eq!(result.matches('•').count(), 3);
        assert_eq!(result.lines().filter(|l| l.contains('•')).count(), 3, "{result:?}");
    }

    #[test]
    fn nested_list_item_indent_is_preserved() {
        let result = render_document("- a\n  - b\n", None, &test_config(80)).unwrap();
        assert!(result.contains("\n  \x1b[1m•\x1b[0m b\n"), "{result:?}");
    }

    #[test]
    fn visible_len_ignores_csi_and_bel_terminated_osc() {
        assert_eq!(visible_len("\x1b[1;34mabc\x1b[0m"), 3);
        assert_eq!(visible_len("\x1b]8;;http://x\x07ab\x1b]8;;\x07"), 2);
    }

    #[test]
    fn blockquote_lines_fit_within_width() {
        let md = "> This quoted paragraph is long enough that it has to wrap onto several lines.";
        let result = render_document(md, None, &test_config(40)).unwrap();
        for line in result.lines() {
            assert!(visible_len(line) <= 40, "line exceeds width: {line:?}");
        }
    }

    #[test]
    fn blockquote_italics_survive_inline_resets() {
        let md = "> before **bold** after";
        let result = render_document(md, None, &test_config(80)).unwrap();
        // Closing bold must not cancel the quote's italics.
        assert!(result.contains("bold\x1b[22m after"), "{result:?}");
        assert!(!result.contains("\x1b[0m after"), "{result:?}");
    }

    #[test]
    fn header_only_table_wider_than_terminal_is_not_dropped() {
        let md = "| alpha | beta | gamma |\n|---|---|---|\n";
        let result = render_document(md, None, &test_config(10)).unwrap();
        for h in ["alpha", "beta", "gamma"] {
            assert!(result.contains(h), "missing {h}: {result:?}");
        }
    }

    #[test]
    fn code_block_in_list_item_is_indented() {
        let md = "- item\n\n  ```\n  code\n  ```\n";
        let result = render_document(md, None, &test_config(80)).unwrap();
        for line in result.lines().filter(|l| l.contains('┌') || l.contains('│') || l.contains('└')) {
            assert!(line.starts_with("  \x1b[2m"), "code block line not indented: {line:?}");
        }
        assert!(result.contains('┌'), "{result:?}");
    }

    #[test]
    fn nested_emphasis_keeps_outer_bold() {
        let result = render_document("**bold *both* still bold**", None, &test_config(80)).unwrap();
        // The inner span must only switch off italics, not reset the surrounding bold.
        assert!(result.contains("both\x1b[23m"), "{result:?}");
        assert!(!result.contains("both\x1b[0m"), "{result:?}");
        assert!(result.ends_with("still bold\x1b[22m\n\n"), "{result:?}");
    }

    #[test]
    fn code_span_inside_bold_keeps_bold() {
        let result = render_document("**a `c` b**", None, &test_config(80)).unwrap();
        assert!(!result.contains("c \x1b[0m"), "{result:?}");
    }

    #[test]
    fn blockquote_italics_survive_nested_emphasis() {
        let result = render_document("> a *b* c", None, &test_config(80)).unwrap();
        assert!(result.contains("\x1b[23m\x1b[3m c"), "{result:?}");
    }

    #[test]
    fn block_quote_has_no_trailing_empty_quote_line() {
        let result = render_document("> one\n\nafter\n", None, &test_config(40)).unwrap();
        let quote_lines = result.lines().filter(|l| l.contains('│')).count();
        assert_eq!(quote_lines, 1, "got: {result:?}");
    }

    #[test]
    fn nested_block_quote_has_no_empty_quote_lines() {
        let result = render_document("> a\n> > b\n", None, &test_config(40)).unwrap();
        for line in result.lines().filter(|l| l.contains('│')) {
            assert!(
                line.chars().any(|c| c.is_alphabetic()),
                "empty quote line in: {result:?}"
            );
        }
    }

    #[test]
    fn thematic_break_in_list_item_fits_within_width() {
        let md = "- item\n\n  ---\n\n  more\n";
        let result = render_document(md, None, &test_config(40)).unwrap();
        for line in result.lines() {
            assert!(visible_len(line) <= 40, "line too wide: {line:?}");
        }
        let rule = result.lines().find(|l| l.contains('─')).expect("rule");
        assert!(rule.starts_with("  \x1b[2m"), "rule not indented: {rule:?}");
    }

    #[test]
    fn top_level_thematic_break_spans_full_width() {
        let result = render_document("---\n", None, &test_config(40)).unwrap();
        assert_eq!(result.matches('─').count(), 40, "{result:?}");
    }

    #[test]
    fn table_in_list_item_is_indented() {
        let md = "- a\n\n  | x | y |\n  |---|---|\n  | 1 | 2 |\n";
        let out = render_document(md, None, &test_config(80)).unwrap();
        let table_lines: Vec<&str> = out.lines().filter(|l| l.contains('│') || l.contains('┌')).collect();
        assert!(!table_lines.is_empty());
        for line in table_lines {
            assert!(line.starts_with("  "), "table line not indented: {line:?}");
        }
    }

    #[test]
    fn table_in_list_item_fits_reduced_width() {
        // 11 columns wide table fits at width 11 but not once indented by 2.
        let md = "- a\n\n  | abcde | fgh |\n  |---|---|\n  | 1 | 2 |\n";
        let out = render_document(md, None, &test_config(14)).unwrap();
        for line in out.lines() {
            assert!(visible_len(line) <= 14, "line too wide: {line:?}");
        }
    }

    #[test]
    fn block_quote_in_list_item_is_indented() {
        let md = "- item\n\n  > quoted\n";
        let out = render_document(md, None, &test_config(80)).unwrap();
        let quote_lines: Vec<&str> = out.lines().filter(|l| l.contains('│')).collect();
        assert_eq!(quote_lines.len(), 1, "{out:?}");
        assert!(quote_lines[0].starts_with("  \x1b[2m│"), "quote line not indented: {out:?}");
    }

    #[test]
    fn block_quote_in_list_item_fits_reduced_width() {
        let md = "- item\n\n  > aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj\n";
        let out = render_document(md, None, &test_config(30)).unwrap();
        for line in out.lines() {
            assert!(visible_len(line) <= 30, "line too wide: {line:?}");
        }
    }
}
