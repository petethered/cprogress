use std::os::unix::io::RawFd;

/// Used when the width cannot be determined — notably under
/// `--progress=force` with stderr redirected.
pub const FALLBACK_WIDTH: usize = 80;

pub fn is_tty(fd: RawFd) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

/// Current terminal width. Re-queried on every repaint, which is how window
/// resizes are handled without a `SIGWINCH` handler.
pub fn width(fd: RawFd) -> Option<usize> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 {
        Some(ws.ws_col as usize)
    } else {
        None
    }
}

pub fn width_or_fallback(fd: RawFd) -> usize {
    width(fd).unwrap_or(FALLBACK_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_fd_is_not_a_tty() {
        assert!(!is_tty(-1));
    }

    #[test]
    fn an_invalid_fd_has_no_width() {
        assert_eq!(width(-1), None);
    }

    #[test]
    fn width_falls_back_to_eighty_columns() {
        assert_eq!(width_or_fallback(-1), FALLBACK_WIDTH);
        assert_eq!(FALLBACK_WIDTH, 80);
    }
}
