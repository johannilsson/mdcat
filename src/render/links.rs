/// Wrap text in an OSC 8 hyperlink escape sequence.
/// This makes links clickable in terminals that support OSC 8
/// (iTerm2, Kitty, WezTerm, GNOME Terminal, etc.)
pub fn osc8_link(url: &str, label: &str) -> String {
    format!(
        "\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\\x1b[2m (→ {url})\x1b[0m",
    )
}
