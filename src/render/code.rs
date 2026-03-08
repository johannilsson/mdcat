use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;

thread_local! {
    static SYNTAX_SET: SyntaxSet = SyntaxSet::load_defaults_newlines();
    static THEME_SET: ThemeSet = ThemeSet::load_defaults();
}

/// Syntax-highlight `code` for the given language using ANSI 24-bit color.
/// Falls back to plain text if the language is unknown.
pub fn highlight_code(code: &str, lang: &str, theme_name: &str) -> String {
    SYNTAX_SET.with(|ss| {
        THEME_SET.with(|ts| {
            let syntax = if lang.is_empty() {
                ss.find_syntax_plain_text()
            } else {
                ss.find_syntax_by_token(lang)
                    .unwrap_or_else(|| ss.find_syntax_plain_text())
            };

            let theme = ts
                .themes
                .get(theme_name)
                .or_else(|| ts.themes.get("base16-ocean.dark"))
                .or_else(|| ts.themes.values().next())
                .expect("no themes loaded");

            let mut highlighter = HighlightLines::new(syntax, theme);
            let mut output = String::new();

            for line in syntect::util::LinesWithEndings::from(code) {
                match highlighter.highlight_line(line, ss) {
                    Ok(ranges) => {
                        output.push_str(&as_24_bit_terminal_escaped(&ranges, false));
                    }
                    Err(_) => {
                        output.push_str(line);
                    }
                }
            }

            // Reset ANSI at end
            output.push_str("\x1b[0m");
            output
        })
    })
}
