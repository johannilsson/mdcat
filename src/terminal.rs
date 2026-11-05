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
/// Writes to /dev/tty directly so it works even when stdout is redirected.
#[cfg(unix)]
fn query_cell_size_csi16t() -> Option<(u32, u32)> {
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
        // CSI 16 t — response: ESC [ 6 ; {height_px} ; {width_px} t
        write!(tty, "\x1b[16t").ok()?;
        tty.flush().ok()?;

        let mut buf = Vec::with_capacity(32);
        let mut byte = [0u8; 1];
        loop {
            match tty.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    buf.push(byte[0]);
                    if byte[0] == b't' {
                        break;
                    }
                    if buf.len() > 64 {
                        break;
                    }
                }
            }
        }

        let s = std::str::from_utf8(&buf).ok()?;
        let s = s.strip_prefix("\x1b[6;")?;
        let s = s.strip_suffix('t')?;
        let (height_str, width_str) = s.split_once(';')?;
        let cell_h: u32 = height_str.parse().ok()?;
        let cell_w: u32 = width_str.parse().ok()?;
        if cell_w > 0 && cell_h > 0 {
            Some((cell_w, cell_h))
        } else {
            None
        }
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
///
/// `override_` is the value of `--image-protocol`; `None` and `"auto"` both
/// mean "detect from the environment".
pub fn detect_image_protocol(override_: Option<&str>) -> ImageProtocol {
    detect_image_protocol_with(override_, |k| std::env::var(k).ok())
}

fn detect_image_protocol_with(
    override_: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> ImageProtocol {
    if let Some(proto) = override_ {
        match proto {
            "auto" => {}
            "kitty" => return ImageProtocol::Kitty,
            "iterm2" => return ImageProtocol::ITerm2,
            "sixel" => return ImageProtocol::Sixel,
            _ => return ImageProtocol::Blocks,
        }
    }

    let term = env("TERM").unwrap_or_default();
    let term_program = env("TERM_PROGRAM").unwrap_or_default();

    // Kitty: $TERM=xterm-kitty
    // Ghostty: $TERM=xterm-ghostty or $TERM_PROGRAM=ghostty (supports Kitty protocol)
    if term == "xterm-kitty" || term == "xterm-ghostty" || term_program == "ghostty" {
        return ImageProtocol::Kitty;
    }

    // iTerm2: $TERM_PROGRAM=iTerm.app or $ITERM_SESSION_ID set
    // WezTerm supports the iTerm2 protocol
    if term_program == "iTerm.app" || term_program == "WezTerm" || env("ITERM_SESSION_ID").is_some() {
        return ImageProtocol::ITerm2;
    }

    // Fall back to Unicode blocks (works everywhere)
    ImageProtocol::Blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn auto_override_detects_from_environment() {
        let env = [("TERM", "xterm-kitty")];
        assert_eq!(detect_image_protocol_with(Some("auto"), env_of(&env)), ImageProtocol::Kitty);
    }

    #[test]
    fn auto_override_falls_back_to_blocks() {
        assert_eq!(detect_image_protocol_with(Some("auto"), env_of(&[])), ImageProtocol::Blocks);
    }

    #[test]
    fn explicit_override_wins_over_environment() {
        let env = [("TERM", "xterm-kitty")];
        assert_eq!(detect_image_protocol_with(Some("iterm2"), env_of(&env)), ImageProtocol::ITerm2);
        assert_eq!(detect_image_protocol_with(Some("sixel"), env_of(&env)), ImageProtocol::Sixel);
        assert_eq!(detect_image_protocol_with(Some("blocks"), env_of(&env)), ImageProtocol::Blocks);
    }

    #[test]
    fn detects_terminals_from_environment() {
        let cases: [(&[(&str, &str)], ImageProtocol); 7] = [
            (&[("TERM", "xterm-ghostty")], ImageProtocol::Kitty),
            (&[("TERM_PROGRAM", "ghostty")], ImageProtocol::Kitty),
            (&[("TERM_PROGRAM", "iTerm.app")], ImageProtocol::ITerm2),
            (&[("ITERM_SESSION_ID", "w0t0p0")], ImageProtocol::ITerm2),
            (&[("TERM_PROGRAM", "WezTerm")], ImageProtocol::ITerm2),
            (&[("TERM", "xterm-256color")], ImageProtocol::Blocks),
            (&[], ImageProtocol::Blocks),
        ];
        for (env, expected) in cases {
            assert_eq!(detect_image_protocol_with(None, env_of(env)), expected, "env: {env:?}");
        }
    }
}
