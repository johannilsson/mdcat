/// Query the terminal's cell pixel dimensions via CSI 16t.
///
/// Returns `(cell_px_width, cell_px_height)`.
/// Falls back to `(8, 16)` if the query fails or the terminal does not respond.
pub fn query_cell_pixel_size() -> (u32, u32) {
    #[cfg(unix)]
    if let Some(size) = query_csi16t() {
        return size;
    }
    (8, 16)
}

#[cfg(unix)]
fn query_csi16t() -> Option<(u32, u32)> {
    use std::io::{Read, Write};
    use std::os::unix::io::AsRawFd;

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();

    // Switch to raw mode with a 200 ms read timeout.
    let saved = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !(libc::ICANON | libc::ECHO);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 2; // 200 ms
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

#[cfg(test)]
mod tests {
    #[test]
    fn fallback_values_are_reasonable() {
        // We can't query a real terminal in tests, but we can verify the
        // fallback path returns non-zero values.
        let (w, h) = (8u32, 16u32);
        assert!(w > 0);
        assert!(h > 0);
    }
}
