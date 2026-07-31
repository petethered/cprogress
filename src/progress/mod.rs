pub mod bar;
pub mod render;

use std::io::Write;
use std::path::Path;

/// Sink for progress events. `engine` talks only to this, never to stderr.
pub trait Progress {
    /// Called during the pre-scan as sources are walked.
    fn scan_tick(&mut self, files: usize, bytes: u64);
    /// Ends the scan phase and begins the copy phase. Only the byte total is
    /// passed: the aggregate line has no file-count component, so a file count
    /// here would be a parameter every implementor ignores.
    fn start(&mut self, total_bytes: u64);
    fn file_started(&mut self, path: &Path, size: u64);
    fn advance(&mut self, bytes: u64);
    /// A diagnostic that must appear above the bar. Goes to stderr, where
    /// `cp` puts its own.
    fn note(&mut self, line: &str);
    /// A `-v` line. Goes to **stdout**, where `cp` puts its own: `cp -v src
    /// dst > log` writes every line into `log`, and so must this. Errors are
    /// `note`'s job and stay on stderr.
    fn verbose(&mut self, line: &str);
    fn finish(&mut self);
}

/// Draws nothing. Used when stderr is not a TTY, or under `--progress=none`.
///
/// Notes and `-v` lines still print: silencing the bar must not swallow them,
/// or a failed copy would look identical to a successful one in a script.
/// The two streams are separate because `cp`'s are.
pub struct Null<W: Write, V: Write> {
    notes: W,
    verbose: V,
}

impl<W: Write, V: Write> Null<W, V> {
    pub fn new(notes: W, verbose: V) -> Self {
        Null { notes, verbose }
    }
}

impl<W: Write, V: Write> Progress for Null<W, V> {
    fn scan_tick(&mut self, _files: usize, _bytes: u64) {}
    fn start(&mut self, _total_bytes: u64) {}
    fn file_started(&mut self, _path: &Path, _size: u64) {}
    fn advance(&mut self, _bytes: u64) {}
    fn note(&mut self, line: &str) {
        let _ = writeln!(self.notes, "{line}");
        let _ = self.notes.flush();
    }
    fn verbose(&mut self, line: &str) {
        let _ = writeln!(self.verbose, "{line}");
        // Flushed per line so it stays interleaved with the notes on stderr
        // when both land on the same terminal.
        let _ = self.verbose.flush();
    }
    fn finish(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn null_emits_nothing_for_progress_events() {
        let mut sink: Vec<u8> = Vec::new();
        let mut verbose: Vec<u8> = Vec::new();
        let mut p = Null::new(&mut sink, &mut verbose);
        p.scan_tick(10, 1000);
        p.start(1000);
        p.file_started(Path::new("a.txt"), 500);
        p.advance(500);
        p.finish();
        assert!(sink.is_empty(), "Null wrote progress output: {sink:?}");
        assert!(verbose.is_empty(), "Null wrote progress output: {verbose:?}");
    }

    #[test]
    fn null_still_emits_notes() {
        // Errors must survive redirection; only the bar is silenced.
        let mut sink: Vec<u8> = Vec::new();
        let mut verbose: Vec<u8> = Vec::new();
        {
            let mut p = Null::new(&mut sink, &mut verbose);
            p.note("cprogress: /x: Permission denied");
        }
        assert_eq!(
            String::from_utf8(sink).expect("utf8"),
            "cprogress: /x: Permission denied\n"
        );
        assert!(verbose.is_empty(), "an error reached the -v stream");
    }

    #[test]
    fn null_keeps_verbose_lines_off_the_note_stream() {
        // cp -v writes to stdout, cp's errors to stderr. They are separate
        // sinks here so main can wire them to the streams cp uses.
        let mut sink: Vec<u8> = Vec::new();
        let mut verbose: Vec<u8> = Vec::new();
        {
            let mut p = Null::new(&mut sink, &mut verbose);
            p.verbose("a.txt -> b.txt");
        }
        assert_eq!(String::from_utf8(verbose).expect("utf8"), "a.txt -> b.txt\n");
        assert!(sink.is_empty(), "a -v line reached the error stream");
    }
}
