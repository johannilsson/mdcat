use anyhow::Result;
use comrak::{Arena, Options, parse_document};
use comrak::nodes::{AstNode, NodeValue, NodeHeading, ListType, NodeCodeBlock};
use std::path::Path;
use crate::render::Config;
use crate::render::code::highlight_code;
use crate::render::images::render_image;
use crate::render::mermaid::render_mermaid;
use crate::render::links::osc8_link;

/// Parse and render a markdown document to an ANSI string.
pub fn render_document(markdown: &str, base_dir: Option<&Path>, config: &Config) -> Result<String> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;

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
                    let line = "═".repeat(width.min(text.len() + 4));
                    output.push_str(&format!("\n\x1b[1;34m{line}\x1b[0m\n"));
                    output.push_str(&format!("\x1b[1;34m  {text}\x1b[0m\n"));
                    output.push_str(&format!("\x1b[1;34m{line}\x1b[0m\n\n"));
                }
                2 => {
                    let line = "─".repeat(width.min(text.len() + 2));
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
            for child in node.children() {
                render_node(child, &mut inner, base_dir, config, 0, false)?;
            }
            for line in inner.lines() {
                output.push_str(&format!("\x1b[2m│\x1b[0m \x1b[3m{line}\x1b[0m\n"));
            }
            output.push('\n');
        }

        NodeValue::CodeBlock(NodeCodeBlock { info, literal, .. }) => {
            let lang = info.split_whitespace().next().unwrap_or("").to_string();
            let code = literal.clone();
            drop(data);

            if lang == "mermaid" && config.mermaid {
                match render_mermaid(&code, config) {
                    Ok(img_str) => output.push_str(&img_str),
                    Err(e) => {
                        eprintln!("mdcat: mermaid render failed: {e}");
                        output.push_str(&render_code_block(&code, "", config));
                        output.push_str("\x1b[2m[Install mmdc to render Mermaid diagrams: npm install -g @mermaid-js/mermaid-cli]\x1b[0m\n\n");
                    }
                }
            } else {
                output.push_str(&render_code_block(&code, &lang, config));
            }
        }

        NodeValue::HtmlBlock(_) => {
            // Skip raw HTML blocks
            drop(data);
        }

        NodeValue::List(list) => {
            let list_type = list.list_type.clone();
            let tight = list.tight;
            drop(data);

            let mut counter = 1u32;
            for child in node.children() {
                let bullet = match list_type {
                    ListType::Bullet => "•".to_string(),
                    ListType::Ordered => {
                        let s = format!("{counter}.");
                        counter += 1;
                        s
                    }
                };
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
            let line = "─".repeat(config.width as usize);
            output.push_str(&format!("\x1b[2m{line}\x1b[0m\n\n"));
        }

        NodeValue::Table(_) => {
            drop(data);
            render_table(node, output, config)?;
        }

        NodeValue::FootnoteDefinition(_) => {
            drop(data);
            // TODO: collect footnotes and render at end
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
            output.push_str("\x1b[0m");
        }

        NodeValue::Emph => {
            drop(data);
            output.push_str("\x1b[3m");
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
            }
            output.push_str("\x1b[0m");
        }

        NodeValue::Strikethrough => {
            drop(data);
            output.push_str("\x1b[9m");
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
            }
            output.push_str("\x1b[0m");
        }

        NodeValue::Code(code) => {
            let text = code.literal.clone();
            drop(data);
            output.push_str(&format!("\x1b[48;5;236m\x1b[96m {text} \x1b[0m"));
        }

        NodeValue::Link(link) => {
            let url = link.url.clone();
            drop(data);
            let mut label = String::new();
            for child in node.children() {
                render_inline(child, &mut label, base_dir, config)?;
            }
            output.push_str(&osc8_link(&url, &label));
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

        NodeValue::TaskItem(checked) => {
            let checked = *checked;
            drop(data);
            let box_char = if checked.is_some() { "☑" } else { "☐" };
            output.push_str(&format!("{box_char} "));
            for child in node.children() {
                render_inline(child, output, base_dir, config)?;
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

    for child in node.children() {
        render_node(child, output, base_dir, config, indent, tight)?;
    }

    Ok(())
}

fn render_code_block(code: &str, lang: &str, config: &Config) -> String {
    let highlighted = highlight_code(code, lang, &config.theme);
    let width = config.width as usize;
    let line = "─".repeat(width.min(60));
    let lang_label = if lang.is_empty() { String::new() } else { format!(" {lang} ") };

    let mut out = String::new();
    out.push_str(&format!("\x1b[2m┌{lang_label}{line}┐\x1b[0m\n"));
    for line_str in highlighted.lines() {
        out.push_str(&format!("\x1b[2m│\x1b[0m {line_str}\n"));
    }
    out.push_str(&format!("\x1b[2m└{line}┘\x1b[0m\n\n"));
    out
}

fn render_table<'a>(node: &'a AstNode<'a>, output: &mut String, _config: &Config) -> Result<()> {
    // Collect rows
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut is_header_row = true;

    for row in node.children() {
        let mut cells: Vec<String> = Vec::new();
        for cell in row.children() {
            let mut cell_text = String::new();
            collect_text(cell, &mut cell_text);
            cells.push(cell_text);
        }
        rows.push(cells);
        if is_header_row {
            is_header_row = false;
        }
    }

    if rows.is_empty() {
        return Ok(());
    }

    // Calculate column widths
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths: Vec<usize> = vec![0; cols];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            col_widths[i] = col_widths[i].max(cell.len());
        }
    }

    let sep: String = col_widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┼");
    let top: String = col_widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┬");
    let bot: String = col_widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┴");

    output.push_str(&format!("\x1b[2m┌{top}┐\x1b[0m\n"));

    for (i, row) in rows.iter().enumerate() {
        let cells: String = row.iter().enumerate().map(|(j, cell)| {
            let width = col_widths.get(j).copied().unwrap_or(0);
            if i == 0 {
                format!(" \x1b[1m{:<width$}\x1b[0m ", cell)
            } else {
                format!(" {:<width$} ", cell)
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
            return;
        }
    }
}

/// Word-wrap text at the given width, accounting for current indent.
fn word_wrap(text: &str, width: usize, indent: usize) -> String {
    // Strip ANSI for width calculation would be complex; use a simple approach
    // that wraps on whitespace boundaries. ANSI codes don't add visual width.
    let available = width.saturating_sub(indent).max(20);
    let mut result = String::new();
    let mut line_len = 0usize;

    for word in text.split_whitespace() {
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

    result
}

/// Estimate visible length of a string, ignoring ANSI escape sequences.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape && ch == 'm' {
            in_escape = false;
        } else if !in_escape {
            len += 1;
        }
    }
    len
}
