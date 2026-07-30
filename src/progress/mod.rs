pub mod bar;
pub mod render;

use std::io::Write;
use std::path::Path;

/// Sink for progress events. `engine` talks only to this, never to stderr.
pub trait Progress {
    /// Called during the pre-scan as sources are walked.
    fn scan_tick(&mut self, files: usize, bytes: u64);
    /// Ends the scan phase and begins the copy phase.
    fn start(&mut self, total_bytes: u64, file_count: usize);
    fn file_started(&mut self, path: &Path, size: u64);
    fn advance(&mut self, bytes: u64);
    /// A line that must appear above the bar: a `-v` entry or an error.
    fn note(&mut self, line: &str);
    fn finish(&mut self);
}

/// Draws nothing. Used when stderr is not a TTY, or under `--progress=none`.
///
/// Notes still print: silencing the bar must not swallow error messages, or a
/// failed copy would look identical to a successful one in a script.
pub struct Null<W: Write> {
    out: W,
}

impl<W: Write> Null<W> {
    pub fn new(out: W) -> Self {
        Null { out }
    }
}

impl<W: Write> Progress for Null<W> {
    fn scan_tick(&mut self, _files: usize, _bytes: u64) {}
    fn start(&mut self, _total_bytes: u64, _file_count: usize) {}
    fn file_started(&mut self, _path: &Path, _size: u64) {}
    fn advance(&mut self, _bytes: u64) {}
    fn note(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
        let _ = self.out.flush();
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
        let mut p = Null::new(&mut sink);
        p.scan_tick(10, 1000);
        p.start(1000, 10);
        p.file_started(Path::new("a.txt"), 500);
        p.advance(500);
        p.finish();
        assert!(sink.is_empty(), "Null wrote progress output: {sink:?}");
    }

    #[test]
    fn null_still_emits_notes() {
        // Errors and -v lines must survive redirection; only the bar is silenced.
        let mut sink: Vec<u8> = Vec::new();
        {
            let mut p = Null::new(&mut sink);
            p.note("cprogress: /x: Permission denied");
        }
        assert_eq!(
            String::from_utf8(sink).expect("utf8"),
            "cprogress: /x: Permission denied\n"
        );
    }
}
