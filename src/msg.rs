//! Diagnostic text shared by every module that writes to stderr.
//!
//! `cp` prefixes each diagnostic with its own name and nothing else, so the
//! prefix and the errno wording live here rather than being rebuilt at each
//! call site. The `cp`-verbatim phrases that are not errno text — "is a
//! directory (not copied).", "are identical (not copied)." and friends — stay
//! at their single use site in `plan`, since there is nothing to share.

use std::ffi::CStr;
use std::io;
use std::path::Path;

/// The name every diagnostic is prefixed with, as `cp` prefixes its own.
pub const PROGRAM: &str = "cprogress";

/// The errno description exactly as `cp` prints it.
///
/// Rust's `io::Error` Display appends `(os error 13)` to the strerror text,
/// which `cp` never does, so the raw errno is translated directly. Errors with
/// no underlying errno — synthesised ones — keep their own Display, which is
/// all there is to say about them.
fn errno_text(err: &io::Error) -> String {
    match err.raw_os_error().and_then(strerror) {
        Some(text) => text,
        None => err.to_string(),
    }
}

/// `strerror_r` rather than `strerror`: the latter may share a static buffer
/// for unrecognised codes, and the test harness runs threads.
///
/// The return code is deliberately ignored in favour of checking the buffer.
/// macOS returns `EINVAL` for an unrecognised errno but still fills the buffer
/// with "Unknown error: N" — which is what `cp` would print. Treating a
/// non-zero return as failure would fall back to `io::Error`'s Display and
/// reintroduce the `(os error N)` suffix this function exists to strip.
fn strerror(code: i32) -> Option<String> {
    let mut buf = [0 as libc::c_char; 256];
    // Safe: the buffer is live for the call and its length is passed honestly.
    // strerror_r NUL-terminates whatever it writes, including on EINVAL.
    unsafe { libc::strerror_r(code, buf.as_mut_ptr(), buf.len()) };
    // Safe: `buf` is NUL-terminated — it was zeroed, and strerror_r terminates.
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
    if text.is_empty() {
        return None;
    }
    Some(text.into_owned())
}

/// `cprogress: <path>: <errno text>` — the shape of every I/O diagnostic
/// `plan` and `engine` emit.
pub fn io_error(path: &Path, err: &io::Error) -> String {
    format!("{PROGRAM}: {}: {}", path.display(), errno_text(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_text_omits_rusts_os_error_suffix() {
        // cp prints "Permission denied", never "Permission denied (os error 13)".
        let err = io::Error::from_raw_os_error(libc::EACCES);
        assert_eq!(errno_text(&err), "Permission denied");
        assert!(
            err.to_string().contains("os error"),
            "test assumption broken: io::Error no longer appends the errno"
        );
    }

    #[test]
    fn errno_text_covers_the_common_copy_failures() {
        assert_eq!(
            errno_text(&io::Error::from_raw_os_error(libc::ENOENT)),
            "No such file or directory"
        );
        assert_eq!(
            errno_text(&io::Error::from_raw_os_error(libc::ENOTDIR)),
            "Not a directory"
        );
        assert_eq!(
            errno_text(&io::Error::from_raw_os_error(libc::EISDIR)),
            "Is a directory"
        );
    }

    #[test]
    fn an_error_without_an_errno_falls_back_to_its_own_display() {
        let err = io::Error::other("made up");
        assert_eq!(errno_text(&err), "made up");
    }

    #[test]
    fn an_unrecognised_errno_still_drops_the_os_error_suffix() {
        // macOS's strerror_r returns EINVAL here but still fills the buffer
        // with "Unknown error: 9999". Treating the return code as failure
        // would fall back to io::Error's Display and put the suffix back.
        let err = io::Error::from_raw_os_error(9999);
        let text = errno_text(&err);
        assert!(
            !text.contains("os error"),
            "the raw errno leaked back in: {text:?}"
        );
        assert!(text.contains("9999"), "the code should still be named: {text:?}");
    }

    #[test]
    fn io_error_builds_the_cp_shaped_line() {
        let err = io::Error::from_raw_os_error(libc::EACCES);
        assert_eq!(
            io_error(Path::new("src/f.txt"), &err),
            "cprogress: src/f.txt: Permission denied"
        );
    }
}
