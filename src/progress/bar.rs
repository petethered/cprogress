use crate::progress::render::{aggregate_line, file_line, scan_line, summary_line};
use crate::progress::Progress;
use crate::rate::{RateEstimator, HALF_LIFE_SECS};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

/// Minimum interval between repaints. Ten frames a second reads as smooth
/// without spending measurable time on terminal writes.
const REPAINT_INTERVAL: Duration = Duration::from_millis(100);

enum Phase {
    Scanning,
    Copying,
    Done,
}

/// The two-line repainting display.
///
/// `out` carries the bar and every diagnostic; it is stderr in practice.
/// `verbose_out` carries `-v` lines only, and is stdout, because that is where
/// `cp` puts them. The two can be the same terminal, so the bar is erased and
/// that erase flushed before anything is written to the other stream.
pub struct Bar<W: Write, V: Write> {
    out: W,
    verbose_out: V,
    width: Box<dyn Fn() -> usize>,
    phase: Phase,

    scan_files: usize,
    scan_bytes: u64,

    total_bytes: u64,
    done_bytes: u64,
    file_path: String,
    file_size: u64,
    file_done: u64,

    rate: RateEstimator,
    started: Instant,
    last_paint: Instant,
    bytes_since_paint: u64,
    lines_drawn: usize,
    repaints: usize,
}

impl<W: Write, V: Write> Bar<W, V> {
    pub fn new(out: W, verbose_out: V, width: Box<dyn Fn() -> usize>) -> Self {
        let now = Instant::now();
        Bar {
            out,
            verbose_out,
            width,
            phase: Phase::Scanning,
            scan_files: 0,
            scan_bytes: 0,
            total_bytes: 0,
            done_bytes: 0,
            file_path: String::new(),
            file_size: 0,
            file_done: 0,
            rate: RateEstimator::new(HALF_LIFE_SECS),
            started: now,
            last_paint: now,
            bytes_since_paint: 0,
            lines_drawn: 0,
            repaints: 0,
        }
    }

    /// Test hook: number of repaints performed so far.
    pub fn repaints(&self) -> usize {
        self.repaints
    }

    fn lines(&self) -> Vec<String> {
        let width = (self.width)();
        match self.phase {
            Phase::Scanning => vec![scan_line(self.scan_files, self.scan_bytes)],
            Phase::Copying => {
                let remaining = self.total_bytes.saturating_sub(self.done_bytes);
                vec![
                    aggregate_line(
                        width,
                        self.done_bytes,
                        self.total_bytes,
                        self.rate.rate(),
                        self.rate.eta(remaining),
                    ),
                    file_line(width, &self.file_path, self.file_done, self.file_size),
                ]
            }
            Phase::Done => Vec::new(),
        }
    }

    /// Erase the currently drawn block, leaving the cursor at column 0.
    fn clear(&mut self) {
        if self.lines_drawn == 0 {
            return;
        }
        let _ = write!(self.out, "\r");
        for _ in 1..self.lines_drawn {
            // Clear this line, then move up to the one above it.
            let _ = write!(self.out, "\x1b[2K\x1b[1A");
        }
        let _ = write!(self.out, "\x1b[2K");
        self.lines_drawn = 0;
    }

    fn draw(&mut self) {
        let lines = self.lines();
        if lines.is_empty() {
            return;
        }
        let _ = write!(self.out, "{}", lines.join("\n"));
        let _ = self.out.flush();
        self.lines_drawn = lines.len();
        self.repaints += 1;
    }

    fn repaint(&mut self, force: bool) {
        let now = Instant::now();
        // The very first paint is never throttled: `last_paint` starts equal
        // to the construction time, so a call arriving within the interval
        // (as any real one will) would otherwise never draw a first frame.
        let never_painted = self.repaints == 0;
        if !force && !never_painted && now.duration_since(self.last_paint) < REPAINT_INTERVAL {
            return;
        }
        let dt = now.duration_since(self.last_paint);
        if matches!(self.phase, Phase::Copying) {
            self.rate.sample(self.bytes_since_paint, dt);
        }
        self.bytes_since_paint = 0;
        self.last_paint = now;
        self.clear();
        self.draw();
    }
}

impl<W: Write, V: Write> Progress for Bar<W, V> {
    fn scan_tick(&mut self, files: usize, bytes: u64) {
        self.scan_files = files;
        self.scan_bytes = bytes;
        self.repaint(false);
    }

    fn start(&mut self, total_bytes: u64, _file_count: usize) {
        self.clear();
        self.phase = Phase::Copying;
        self.total_bytes = total_bytes;
        self.started = Instant::now();
        self.last_paint = self.started;
        self.repaint(true);
    }

    fn file_started(&mut self, path: &Path, size: u64) {
        self.file_path = path.to_string_lossy().into_owned();
        self.file_size = size;
        self.file_done = 0;
        self.repaint(true);
    }

    fn advance(&mut self, bytes: u64) {
        self.done_bytes = self.done_bytes.saturating_add(bytes);
        self.file_done = self.file_done.saturating_add(bytes);
        self.bytes_since_paint = self.bytes_since_paint.saturating_add(bytes);
        self.repaint(false);
    }

    fn note(&mut self, line: &str) {
        self.clear();
        let _ = writeln!(self.out, "{line}");
        // draw() flushes, so the line reaches the terminal with the redraw.
        self.draw();
    }

    fn verbose(&mut self, line: &str) {
        self.clear();
        // The erase must land before the other stream writes, or the -v line
        // is printed into the columns the bar still occupies. draw()'s own
        // flush comes too late for that, so flush here.
        let _ = self.out.flush();
        let _ = writeln!(self.verbose_out, "{line}");
        let _ = self.verbose_out.flush();
        self.draw();
    }

    fn finish(&mut self) {
        self.clear();
        self.phase = Phase::Done;
        let elapsed = self.started.elapsed();
        let average = if elapsed.as_secs_f64() > 0.0 {
            Some(self.done_bytes as f64 / elapsed.as_secs_f64())
        } else {
            None
        };
        let _ = writeln!(
            self.out,
            "{}",
            summary_line(self.done_bytes, elapsed, average)
        );
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Progress;
    use std::path::Path;

    const MB: u64 = 1024 * 1024;

    fn bar_with(sink: &mut Vec<u8>, width: usize) -> Bar<&mut Vec<u8>, std::io::Sink> {
        Bar::new(sink, std::io::sink(), Box::new(move || width))
    }

    fn bar_with_verbose<'a>(
        sink: &'a mut Vec<u8>,
        verbose: &'a mut Vec<u8>,
        width: usize,
    ) -> Bar<&'a mut Vec<u8>, &'a mut Vec<u8>> {
        Bar::new(sink, verbose, Box::new(move || width))
    }

    fn rendered(sink: Vec<u8>) -> String {
        String::from_utf8(sink).expect("utf8")
    }

    #[test]
    fn scan_tick_draws_the_scanning_line() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 80);
            b.scan_tick(41_203, 1024);
        }
        let out = rendered(sink);
        assert!(out.contains("Scanning..."), "got {out:?}");
        assert!(out.contains("41,203 files"), "got {out:?}");
    }

    #[test]
    fn start_then_file_draws_both_lines() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 80);
            b.start(100 * MB, 2);
            b.file_started(Path::new("big.bin"), 50 * MB);
        }
        let out = rendered(sink);
        assert!(out.contains("total"), "missing aggregate line: {out:?}");
        assert!(out.contains("big.bin"), "missing file line: {out:?}");
        assert!(out.contains('\n'), "expected two lines: {out:?}");
    }

    #[test]
    fn advance_is_throttled_between_repaints() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 80);
            b.start(100 * MB, 1);
            b.file_started(Path::new("big.bin"), 100 * MB);
            let after_setup = b.repaints();
            for _ in 0..1000 {
                b.advance(1024);
            }
            assert!(
                b.repaints() - after_setup <= 2,
                "throttle let {} repaints through",
                b.repaints() - after_setup
            );
        }
    }

    #[test]
    fn note_writes_above_the_bar_and_redraws() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 80);
            b.start(100 * MB, 1);
            b.file_started(Path::new("big.bin"), 100 * MB);
            b.note("cprogress: /x: Permission denied");
        }
        let out = rendered(sink);
        assert!(out.contains("Permission denied"), "note missing: {out:?}");
        // The bar is redrawn after the note, so the aggregate label appears again.
        let label_count = out.matches("total ").count();
        assert!(label_count >= 2, "bar not redrawn after note: {out:?}");
    }

    #[test]
    fn verbose_writes_to_the_other_stream_and_redraws() {
        // The line goes to stdout, as cp's does; the bar stays on stderr.
        let mut sink = Vec::new();
        let mut verbose = Vec::new();
        {
            let mut b = bar_with_verbose(&mut sink, &mut verbose, 80);
            b.start(100 * MB, 1);
            b.file_started(Path::new("big.bin"), 100 * MB);
            b.verbose("src -> dst");
        }
        assert_eq!(rendered(verbose), "src -> dst\n");
        let out = rendered(sink);
        assert!(!out.contains("src -> dst"), "-v leaked onto the bar stream: {out:?}");
        // The bar stream still carries the erase and the redraw.
        assert!(out.contains("\x1b[2K"), "bar was not cleared: {out:?}");
        assert!(
            out.matches("total ").count() >= 2,
            "bar not redrawn after a -v line: {out:?}"
        );
    }

    #[test]
    fn finish_clears_the_bar_and_prints_a_summary() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 80);
            b.start(10 * MB, 1);
            b.file_started(Path::new("big.bin"), 10 * MB);
            b.advance(10 * MB);
            b.finish();
        }
        let out = rendered(sink);
        assert!(out.contains("copied in"), "no summary: {out:?}");
        assert!(out.ends_with('\n'), "summary not newline-terminated: {out:?}");
    }

    #[test]
    fn a_second_file_replaces_the_first_in_the_file_line() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 80);
            b.start(20 * MB, 2);
            b.file_started(Path::new("first.bin"), 10 * MB);
            b.advance(10 * MB);
            b.file_started(Path::new("second.bin"), 10 * MB);
        }
        let out = rendered(sink);
        assert!(out.contains("second.bin"), "got {out:?}");
    }

    #[test]
    fn writes_never_exceed_the_terminal_width_per_line() {
        let mut sink = Vec::new();
        {
            let mut b = bar_with(&mut sink, 40);
            b.start(100 * MB, 1);
            b.file_started(Path::new("some/deep/path/to/a/file.bin"), 100 * MB);
        }
        let out = rendered(sink);
        for line in out.split('\n') {
            let visible: String = strip_ansi(line);
            assert!(
                visible.chars().count() <= 40,
                "line exceeded width 40: {visible:?}"
            );
        }
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else if c != '\r' {
                out.push(c);
            }
        }
        out
    }
}
