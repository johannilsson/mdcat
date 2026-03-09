use terminal_size::{terminal_size, Height, Width};

/// Detect the current terminal width, defaulting to 80.
pub fn width() -> u16 {
    terminal_size()
        .map(|(Width(w), _)| w)
        .unwrap_or(80)
}

/// Detect the current terminal height in lines, defaulting to 24.
pub fn height() -> u16 {
    terminal_size()
        .map(|(_, Height(h))| h)
        .unwrap_or(24)
}

/// Returns true if stdout is connected to a TTY.
pub fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// Detected terminal graphics protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageProtocol {
    Kitty,
    ITerm2,
    Sixel,
    Blocks, // Unicode half-block fallback
}

/// Auto-detect the best image protocol supported by the current terminal.
pub fn detect_image_protocol(override_: Option<&str>) -> ImageProtocol {
    if let Some(proto) = override_ {
        return match proto {
            "kitty" => ImageProtocol::Kitty,
            "iterm2" => ImageProtocol::ITerm2,
            "sixel" => ImageProtocol::Sixel,
            _ => ImageProtocol::Blocks,
        };
    }

    // Kitty: $TERM=xterm-kitty
    // Ghostty: $TERM=xterm-ghostty or $TERM_PROGRAM=ghostty (supports Kitty protocol)
    if std::env::var("TERM").map(|t| t == "xterm-kitty" || t == "xterm-ghostty").unwrap_or(false)
        || std::env::var("TERM_PROGRAM").map(|t| t == "ghostty").unwrap_or(false)
    {
        return ImageProtocol::Kitty;
    }

    // iTerm2: $TERM_PROGRAM=iTerm.app or $ITERM_SESSION_ID set
    if std::env::var("TERM_PROGRAM").map(|t| t == "iTerm.app").unwrap_or(false)
        || std::env::var("ITERM_SESSION_ID").is_ok()
    {
        return ImageProtocol::ITerm2;
    }

    // WezTerm supports iTerm2 protocol
    if std::env::var("TERM_PROGRAM").map(|t| t == "WezTerm").unwrap_or(false) {
        return ImageProtocol::ITerm2;
    }

    // Fall back to Unicode blocks (works everywhere)
    ImageProtocol::Blocks
}
