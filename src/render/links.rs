/// Wrap text in an OSC 8 hyperlink escape sequence.
/// This makes links clickable in terminals that support OSC 8
/// (iTerm2, Kitty, WezTerm, GNOME Terminal, etc.)
pub fn osc8_link(url: &str, label: &str) -> String {
    format!(
        "\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\\x1b[2m (→ {url})\x1b[0m",
    )
}

/// Like [`osc8_link`], but omits the "(→ url)" suffix when the visible link
/// text already is the URL (autolinks such as `<https://x.com>` or
/// `<me@x.com>`), where repeating it would only add noise.
pub fn osc8_link_for(url: &str, label: &str, plain_label: &str) -> String {
    let same = plain_label == url || url.strip_prefix("mailto:") == Some(plain_label);
    if same {
        format!("\x1b]8;;{url}\x1b\\{label}\x1b]8;;\x1b\\")
    } else {
        osc8_link(url, label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_shown_when_label_differs() {
        let s = osc8_link_for("http://a", "x", "x");
        assert!(s.contains("(→ http://a)"));
    }

    #[test]
    fn suffix_omitted_when_label_is_url() {
        let s = osc8_link_for("http://a", "http://a", "http://a");
        assert!(!s.contains('→'));
        assert!(s.contains("\x1b]8;;http://a\x1b\\http://a\x1b]8;;\x1b\\"));
    }

    #[test]
    fn suffix_omitted_for_mailto_autolink() {
        let s = osc8_link_for("mailto:me@x.com", "me@x.com", "me@x.com");
        assert!(!s.contains('→'));
    }
}
