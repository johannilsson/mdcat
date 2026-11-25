use terminal_size::{terminal_size, Height, Width};

/// Detect the current terminal width, defaulting to 80.
pub fn width() -> u16 {
    terminal_size()
        .map(|(Width(w), _)| w)
        .unwrap_or(80)
}

/// Detect the current terminal height, defaulting to 24.
pub fn height() -> u16 {
    terminal_size()
        .map(|(_, Height(h))| h)
        .unwrap_or(24)
}

/// Returns the terminal's cell pixel width.
///
/// Queries via CSI 16 t (Report Character Cell Size in Pixels), which is
/// supported by iTerm2, Kitty, Ghostty, and WezTerm. Falls back to
/// TIOCGWINSZ pixel fields, then to a reasonable default of 10.
pub fn cell_pixel_width() -> u32 {
    #[cfg(unix)]
    if let Some((w, _)) = query_cell_size_csi16t() {
        return w;
    }
    #[cfg(unix)]
    if let Some(w) = query_cell_width_tiocgwinsz() {
        return w;
    }
    10
}

/// Returns the terminal's cell pixel height.
///
/// Queries via CSI 16 t (Report Character Cell Size in Pixels). Falls back to 20.
pub fn cell_pixel_height() -> u32 {
    #[cfg(unix)]
    if let Some((_, h)) = query_cell_size_csi16t() {
        return h;
    }
    20
}

/// Query cell pixel dimensions using the CSI 16 t escape sequence.
///
/// Returns `(cell_px_width, cell_px_height)`.
#[cfg(unix)]
fn query_cell_size_csi16t() -> Option<(u32, u32)> {
    // Response: ESC [ 6 ; {height_px} ; {width_px} t
    let reply = query_tty(b"\x1b[16t", |buf| buf.ends_with(b"t"))?;
    parse_cell_size(&reply)
}

fn parse_cell_size(reply: &[u8]) -> Option<(u32, u32)> {
    let s = std::str::from_utf8(reply).ok()?;
    let s = s.strip_prefix("\x1b[6;")?.strip_suffix('t')?;
    let (height_str, width_str) = s.split_once(';')?;
    let cell_h: u32 = height_str.parse().ok()?;
    let cell_w: u32 = width_str.parse().ok()?;
    (cell_w > 0 && cell_h > 0).then_some((cell_w, cell_h))
}

/// Send `request` to the controlling terminal and read its reply until `done`
/// says the bytes so far are complete, the terminal stays quiet for 200ms, or
/// the reply grows past 256 bytes.
///
/// Talks to /dev/tty directly so it works even when stdout is redirected.
#[cfg(unix)]
fn query_tty(request: &[u8], done: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();

    // Switch to raw mode with a 200ms read timeout
    let saved = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !(libc::ICANON | libc::ECHO);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 2; // 200ms
        libc::tcsetattr(fd, libc::TCSANOW, &t);
        saved
    };

    let result = (|| {
        tty.write_all(request).ok()?;
        tty.flush().ok()?;

        let mut buf = Vec::with_capacity(32);
        let mut byte = [0u8; 1];
        while let Ok(1) = tty.read(&mut byte) {
            buf.push(byte[0]);
            if done(&buf) || buf.len() > 256 {
                break;
            }
        }
        Some(buf)
    })();

    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
    result
}

/// Fall back to TIOCGWINSZ pixel fields (not populated by all terminals).
#[cfg(unix)]
fn query_cell_width_tiocgwinsz() -> Option<u32> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_xpixel > 0
            && ws.ws_col > 0
        {
            Some(ws.ws_xpixel as u32 / ws.ws_col as u32)
        } else {
            None
        }
    }
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

    // Resolved once: the query below talks to the terminal, which must not
    // happen while a pager owns it (images are detected again per image and
    // on every live refresh).
    static DETECTED: std::sync::OnceLock<ImageProtocol> = std::sync::OnceLock::new();
    DETECTED
        .get_or_init(|| match detect_from_env(|name| std::env::var(name).ok()) {
            ImageProtocol::Blocks => query_kitty_support().unwrap_or(ImageProtocol::Blocks),
            known => known,
        })
        .clone()
}

/// Ask the terminal whether it speaks the Kitty graphics protocol, for
/// terminals the environment doesn't identify (`TERM_PROGRAM` unknown, or
/// lost over ssh or tmux).
///
/// Sends a graphics query (`a=q`, which draws nothing) followed by a Primary
/// Device Attributes request. A terminal with Kitty graphics answers the first
/// with `OK`; every terminal answers the second, so waiting for it means we
/// don't have to wait out the timeout when the first goes unanswered.
#[cfg(unix)]
fn query_kitty_support() -> Option<ImageProtocol> {
    use std::io::IsTerminal;

    if !std::io::stdout().is_terminal() {
        return None;
    }
    let reply = query_tty(
        b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c",
        // The device attributes reply ends the exchange: ESC [ ? ... c
        |buf| buf.ends_with(b"c") && buf.windows(3).any(|w| w == b"\x1b[?"),
    )?;
    supports_kitty_graphics(&reply).then_some(ImageProtocol::Kitty)
}

#[cfg(not(unix))]
fn query_kitty_support() -> Option<ImageProtocol> {
    None
}

/// Whether `reply` contains the acknowledgement of the graphics query.
fn supports_kitty_graphics(reply: &[u8]) -> bool {
    reply.windows(12).any(|w| w == b"\x1b_Gi=31;OK\x1b\\")
}

/// The protocol implied by the terminal's environment variables.
fn detect_from_env(var: impl Fn(&str) -> Option<String>) -> ImageProtocol {
    let term = var("TERM");
    let program = var("TERM_PROGRAM");
    let term = term.as_deref();
    let program = program.as_deref();

    // Kitty: $TERM=xterm-kitty
    // Ghostty: $TERM=xterm-ghostty or $TERM_PROGRAM=ghostty (supports Kitty protocol)
    // Rex: $TERM_PROGRAM=rex (supports Kitty protocol, but sets TERM=xterm-256color)
    if matches!(term, Some("xterm-kitty" | "xterm-ghostty"))
        || matches!(program, Some("ghostty" | "rex"))
    {
        return ImageProtocol::Kitty;
    }

    // iTerm2: $TERM_PROGRAM=iTerm.app or $ITERM_SESSION_ID set
    // WezTerm supports the iTerm2 protocol
    if matches!(program, Some("iTerm.app" | "WezTerm")) || var("ITERM_SESSION_ID").is_some() {
        return ImageProtocol::ITerm2;
    }

    // Fall back to Unicode blocks (works everywhere)
    ImageProtocol::Blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(vars: &[(&str, &str)]) -> ImageProtocol {
        detect_from_env(|name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn kitty_family_is_detected_from_term() {
        assert_eq!(detect(&[("TERM", "xterm-kitty")]), ImageProtocol::Kitty);
        assert_eq!(detect(&[("TERM", "xterm-ghostty")]), ImageProtocol::Kitty);
    }

    #[test]
    fn ghostty_and_rex_are_detected_from_term_program() {
        assert_eq!(detect(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "ghostty")]), ImageProtocol::Kitty);
        assert_eq!(detect(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "rex")]), ImageProtocol::Kitty);
    }

    #[test]
    fn iterm2_and_wezterm_use_the_iterm2_protocol() {
        assert_eq!(detect(&[("TERM_PROGRAM", "iTerm.app")]), ImageProtocol::ITerm2);
        assert_eq!(detect(&[("TERM_PROGRAM", "WezTerm")]), ImageProtocol::ITerm2);
        assert_eq!(detect(&[("ITERM_SESSION_ID", "w0t0p0")]), ImageProtocol::ITerm2);
    }

    #[test]
    fn unknown_terminals_fall_back_to_blocks() {
        assert_eq!(detect(&[("TERM", "xterm-256color")]), ImageProtocol::Blocks);
        assert_eq!(detect(&[]), ImageProtocol::Blocks);
    }

    #[test]
    fn kitty_graphics_acknowledgement_is_recognised() {
        assert!(supports_kitty_graphics(b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22c"));
        // A terminal without it only answers the device attributes request.
        assert!(!supports_kitty_graphics(b"\x1b[?62;22c"));
        assert!(!supports_kitty_graphics(b"\x1b_Gi=31;ENOTSUPPORTED:no\x1b\\"));
        assert!(!supports_kitty_graphics(b""));
    }

    #[test]
    fn cell_size_reply_is_parsed() {
        assert_eq!(parse_cell_size(b"\x1b[6;20;10t"), Some((10, 20)));
        assert_eq!(parse_cell_size(b"\x1b[6;0;0t"), None);
        assert_eq!(parse_cell_size(b"garbage"), None);
        assert_eq!(parse_cell_size(b""), None);
    }

    #[test]
    fn override_wins_over_the_environment() {
        assert_eq!(detect_image_protocol(Some("blocks")), ImageProtocol::Blocks);
        assert_eq!(detect_image_protocol(Some("kitty")), ImageProtocol::Kitty);
    }
}
