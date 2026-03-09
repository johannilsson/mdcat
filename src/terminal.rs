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

/// Returns the terminal's cell pixel width.
///
/// Queries via CSI 16 t (Report Character Cell Size in Pixels), which is
/// supported by iTerm2, Kitty, Ghostty, and WezTerm. Falls back to
/// TIOCGWINSZ pixel fields, then to a reasonable default of 10.
pub fn cell_pixel_width() -> u32 {
    #[cfg(unix)]
    if let Some(w) = query_cell_width_csi16t() {
        return w;
    }
    #[cfg(unix)]
    if let Some(w) = query_cell_width_tiocgwinsz() {
        return w;
    }
    10
}

/// Query cell pixel width using CSI 16 t escape sequence.
/// Writes to /dev/tty directly so it works even when stdout is redirected.
#[cfg(unix)]
fn query_cell_width_csi16t() -> Option<u32> {
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
        // CSI 16 t — response: ESC [ 6 ; {height} ; {width} t
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
        let (_, width_str) = s.split_once(';')?;
        let width: u32 = width_str.parse().ok()?;
        if width > 0 { Some(width) } else { None }
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
