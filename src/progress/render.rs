use crate::fmt::{human_bytes, human_count, human_duration, human_eta, human_rate};
use std::time::Duration;

/// Files smaller than this get no per-file bar. Below the threshold a file
/// completes inside a single copy chunk, so the bar would only ever show 0% or
/// 100% and would strobe at the repaint rate instead of informing.
pub const FILE_BAR_MIN_BYTES: u64 = 8 * 1024 * 1024;

/// A bar narrower than this is not worth the columns; the line drops it and
/// shows text only.
pub const MIN_BAR_COLUMNS: usize = 8;

const AGGREGATE_LABEL: &str = "total";

/// Render `cols` columns of bar for a completion fraction.
pub fn bar(cols: usize, frac: f64) -> String {
    if cols == 0 {
        return String::new();
    }
    let frac = frac.clamp(0.0, 1.0);
    let filled = ((cols as f64) * frac).floor() as usize;
    let filled = filled.min(cols);
    let mut s = String::with_capacity(cols);
    if filled >= cols {
        s.push_str(&"=".repeat(cols));
    } else if filled > 0 {
        s.push_str(&"=".repeat(filled - 1));
        s.push('>');
    }
    while s.chars().count() < cols {
        s.push(' ');
    }
    s
}

/// Shorten to `max` characters keeping both ends, e.g. `photos/…/IMG.HEIC`.
pub fn middle_ellipsize(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max <= 3 {
        return chars[..max].iter().collect();
    }
    let keep = max - 1;
    let head = keep - keep / 2;
    let tail = keep - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(chars[chars.len() - tail..].iter());
    out
}

fn percent(done: u64, total: u64) -> u64 {
    if total == 0 {
        return 100;
    }
    // Clamped because the total can be stale if the tree changed mid-copy.
    ((done as f64 / total as f64) * 100.0).floor().min(100.0) as u64
}

fn fraction(done: u64, total: u64) -> f64 {
    if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    }
}

fn truncate_to(s: String, width: usize) -> String {
    if s.chars().count() <= width {
        s
    } else {
        s.chars().take(width).collect()
    }
}

/// `total  62%[===========>      ] 4.21G  118MB/s  eta 22s`
pub fn aggregate_line(
    width: usize,
    done: u64,
    total: u64,
    rate: Option<f64>,
    eta: Option<Duration>,
) -> String {
    let head = format!("{AGGREGATE_LABEL} {:>3}%", percent(done, total));
    let tail = format!(
        " {}  {}  {}",
        human_bytes(done),
        human_rate(rate),
        human_eta(eta)
    );
    let fixed = head.chars().count() + tail.chars().count() + 2; // 2 for the brackets
    let bar_cols = width.saturating_sub(fixed);
    if bar_cols < MIN_BAR_COLUMNS {
        return truncate_to(format!("{head}{tail}"), width);
    }
    format!("{head}[{}]{tail}", bar(bar_cols, fraction(done, total)))
}

/// `IMG_4471.HEIC  30%[====>          ] 1.4M/4.6M`, or a bare path below the
/// size threshold.
pub fn file_line(width: usize, path: &str, done: u64, size: u64) -> String {
    if size < FILE_BAR_MIN_BYTES {
        return truncate_to(middle_ellipsize(path, width), width);
    }
    let tail = format!(" {}/{}", human_bytes(done), human_bytes(size));
    let pct = format!(" {:>3}%", percent(done, size));
    // Brackets plus the narrowest bar worth drawing.
    let overhead = pct.chars().count() + tail.chars().count() + 2;
    let name_budget = width.saturating_sub(overhead + MIN_BAR_COLUMNS);
    if name_budget == 0 {
        return truncate_to(middle_ellipsize(path, width), width);
    }
    let name = middle_ellipsize(path, name_budget);
    // `name_budget` was width - (overhead + MIN_BAR_COLUMNS) and is non-zero,
    // so the subtraction did not saturate and `name` is at most that many
    // characters. `bar_cols` is therefore always at least MIN_BAR_COLUMNS
    // here — no need for the aggregate line's drop-the-bar fallback.
    let bar_cols = width.saturating_sub(name.chars().count() + overhead);
    format!("{name}{pct}[{}]{tail}", bar(bar_cols, fraction(done, size)))
}

/// `big.bin  62%[===========>      ] 4.21G/6.8G  118MB/s  eta 22s`
///
/// Used when the whole operation is a single file. The aggregate line and the
/// per-file line would carry identical numbers, so they collapse into one that
/// names the file and owns the rate and ETA.
///
/// Unlike [`file_line`], this ignores [`FILE_BAR_MIN_BYTES`]: that threshold
/// exists to stop the per-file line strobing as a tree of small files flies
/// past, and with a single file there is nothing to strobe against.
pub fn single_line(
    width: usize,
    path: &str,
    done: u64,
    total: u64,
    rate: Option<f64>,
    eta: Option<Duration>,
) -> String {
    let tail = format!(
        " {}/{}  {}  {}",
        human_bytes(done),
        human_bytes(total),
        human_rate(rate),
        human_eta(eta)
    );
    let pct = format!(" {:>3}%", percent(done, total));
    // Brackets plus the narrowest bar worth drawing.
    let overhead = pct.chars().count() + tail.chars().count() + 2;
    let name_budget = width.saturating_sub(overhead + MIN_BAR_COLUMNS);
    if name_budget == 0 {
        // No room for a name plus a bar; fall back to the numbers alone, which
        // are what the user is actually watching.
        return truncate_to(format!("{}{pct}{tail}", middle_ellipsize(path, 0)), width);
    }
    let name = middle_ellipsize(path, name_budget);
    let bar_cols = width.saturating_sub(name.chars().count() + overhead);
    format!("{name}{pct}[{}]{tail}", bar(bar_cols, fraction(done, total)))
}

/// `Scanning... 41,203 files, 88.2G`
pub fn scan_line(files: usize, bytes: u64) -> String {
    format!(
        "Scanning... {} {}, {}",
        human_count(files),
        if files == 1 { "file" } else { "files" },
        human_bytes(bytes)
    )
}

/// `88.2G copied in 12m41s (118MB/s)`
pub fn summary_line(total: u64, elapsed: Duration, rate: Option<f64>) -> String {
    format!(
        "{} copied in {} ({})",
        human_bytes(total),
        human_duration(elapsed),
        human_rate(rate)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    #[test]
    fn bar_fills_proportionally_with_an_arrow_head() {
        assert_eq!(bar(10, 0.0), "          ");
        assert_eq!(bar(10, 0.5), "====>     ");
        assert_eq!(bar(10, 1.0), "==========");
    }

    #[test]
    fn bar_clamps_out_of_range_fractions() {
        assert_eq!(bar(4, -1.0), "    ");
        assert_eq!(bar(4, 2.0), "====");
        assert_eq!(bar(0, 0.5), "");
    }

    #[test]
    fn middle_ellipsize_keeps_both_ends() {
        assert_eq!(middle_ellipsize("short", 10), "short");
        assert_eq!(middle_ellipsize("abcdefghij", 10), "abcdefghij");
        // The brief's algorithm (keep = max - 1; head = keep - keep / 2) splits
        // the remaining characters evenly, biasing extra to the tail on odd
        // counts: keep=4 gives head=2, tail=2, not the head=3/tail=1 the brief's
        // prose example implied. Asserting the real, non-overflowing output.
        assert_eq!(middle_ellipsize("abcdefghijk", 5), "ab…jk");
        assert_eq!(middle_ellipsize("abcdefghijk", 0), "");
    }

    #[test]
    fn middle_ellipsize_counts_characters_not_bytes() {
        let s = "ααααααααααββββββββββ"; // 20 chars, 40 bytes
        let out = middle_ellipsize(s, 9);
        assert_eq!(out.chars().count(), 9);
    }

    #[test]
    fn aggregate_line_matches_the_specified_layout() {
        let line = aggregate_line(
            56,
            (4.21 * GB as f64) as u64,
            (6.8 * GB as f64) as u64,
            Some(118.0 * MB as f64),
            Some(Duration::from_secs(22)),
        );
        assert!(line.starts_with("total "), "got {line:?}");
        assert!(line.contains('%'), "got {line:?}");
        assert!(line.contains("118MB/s"), "got {line:?}");
        assert!(line.contains("eta 22s"), "got {line:?}");
        assert!(line.chars().count() <= 56, "overflowed width: {line:?}");
    }

    #[test]
    fn aggregate_line_fits_every_width() {
        for width in [20usize, 40, 56, 80, 200] {
            let line = aggregate_line(width, 4 * GB, 8 * GB, Some(100.0 * MB as f64), None);
            assert!(
                line.chars().count() <= width,
                "width {width} overflowed: {line:?}"
            );
        }
    }

    #[test]
    fn aggregate_line_drops_the_bar_when_space_runs_out() {
        let line = aggregate_line(24, 4 * GB, 8 * GB, Some(100.0 * MB as f64), None);
        assert!(!line.contains('['), "narrow line kept a bar: {line:?}");
        assert!(line.chars().count() <= 24);
    }

    #[test]
    fn a_zero_byte_total_renders_as_complete() {
        let line = aggregate_line(80, 0, 0, None, None);
        assert!(line.contains("100%"), "got {line:?}");
    }

    #[test]
    fn aggregate_percentage_clamps_when_the_tree_grew_mid_copy() {
        let line = aggregate_line(80, 200, 100, None, None);
        assert!(line.contains("100%"), "got {line:?}");
    }

    #[test]
    fn file_line_shows_a_bar_at_or_above_the_threshold() {
        let line = file_line(80, "photos/IMG_4471.HEIC", 3 * MB, 10 * MB);
        assert!(line.contains('['), "expected a bar: {line:?}");
        assert!(line.contains("10.0M"), "expected the total: {line:?}");
        assert!(line.chars().count() <= 80);
    }

    #[test]
    fn file_line_is_a_bare_path_below_the_threshold() {
        let line = file_line(80, "notes/todo.txt", 0, 1024);
        assert_eq!(line, "notes/todo.txt");
    }

    #[test]
    fn the_threshold_is_eight_mebibytes() {
        assert_eq!(FILE_BAR_MIN_BYTES, 8 * 1024 * 1024);
        assert!(!file_line(80, "f", 0, FILE_BAR_MIN_BYTES).is_empty());
        assert!(file_line(80, "f", 0, FILE_BAR_MIN_BYTES).contains('['));
        assert!(!file_line(80, "f", 0, FILE_BAR_MIN_BYTES - 1).contains('['));
    }

    #[test]
    fn file_line_truncates_long_paths_to_fit() {
        let long = "a/".repeat(100) + "final.bin";
        let line = file_line(80, &long, MB, 100 * MB);
        assert!(line.chars().count() <= 80, "overflowed: {line:?}");
        assert!(line.contains('…'), "expected ellipsis: {line:?}");
    }

    #[test]
    fn file_line_fits_every_width() {
        for width in [20usize, 40, 80, 200] {
            let line = file_line(width, "some/dir/file.bin", MB, 100 * MB);
            assert!(
                line.chars().count() <= width,
                "width {width} overflowed: {line:?}"
            );
        }
    }

    #[test]
    fn single_line_carries_name_progress_rate_and_eta() {
        let line = single_line(
            80,
            "big.bin",
            3 * GB,
            6 * GB,
            Some(118.0 * MB as f64),
            Some(Duration::from_secs(22)),
        );
        assert!(line.starts_with("big.bin"), "got {line:?}");
        assert!(line.contains(" 50%"), "got {line:?}");
        assert!(line.contains("3.00G/6.00G"), "got {line:?}");
        assert!(line.contains("118MB/s"), "got {line:?}");
        assert!(line.contains("eta 22s"), "got {line:?}");
        assert!(line.contains('['), "expected a bar: {line:?}");
        assert!(line.chars().count() <= 80, "overflowed: {line:?}");
    }

    #[test]
    fn single_line_shows_a_bar_below_the_per_file_threshold() {
        // FILE_BAR_MIN_BYTES exists to stop the per-file line strobing as many
        // small files fly past. With one file there is nothing to strobe
        // against, so a small file still gets a bar and an ETA.
        // Even, so the halfway point lands on exactly 50% rather than flooring.
        let small = FILE_BAR_MIN_BYTES - 2;
        assert!(small < FILE_BAR_MIN_BYTES, "fixture must be under the threshold");
        assert!(
            !file_line(80, "notes.txt", small / 2, small).contains('['),
            "fixture should get no bar from the multi-file layout"
        );
        let line = single_line(80, "notes.txt", small / 2, small, None, None);
        assert!(line.contains('['), "expected a bar: {line:?}");
        assert!(line.contains(" 50%"), "got {line:?}");
    }

    #[test]
    fn single_line_fits_every_width() {
        let long = "a/".repeat(100) + "final.bin";
        for width in [10usize, 20, 40, 56, 80, 200] {
            for path in ["f.bin", long.as_str()] {
                let line = single_line(
                    width,
                    path,
                    GB,
                    4 * GB,
                    Some(100.0 * MB as f64),
                    Some(Duration::from_secs(30)),
                );
                assert!(
                    line.chars().count() <= width,
                    "width {width} overflowed with {path:?}: {line:?}"
                );
            }
        }
    }

    #[test]
    fn single_line_handles_a_zero_byte_total() {
        let line = single_line(80, "empty.txt", 0, 0, None, None);
        assert!(line.contains("100%"), "got {line:?}");
        assert!(line.chars().count() <= 80, "overflowed: {line:?}");
    }

    #[test]
    fn scan_line_reports_files_and_bytes() {
        let line = scan_line(41_203, (88.2 * GB as f64) as u64);
        assert!(line.starts_with("Scanning..."), "got {line:?}");
        assert!(line.contains("41,203 files"), "got {line:?}");
        assert!(line.contains("88.2G"), "got {line:?}");
    }

    #[test]
    fn scan_line_says_one_file_not_one_files() {
        assert!(scan_line(1, 1024).contains("1 file,"), "{}", scan_line(1, 1024));
        assert!(scan_line(0, 0).contains("0 files,"), "{}", scan_line(0, 0));
        assert!(scan_line(2, 0).contains("2 files,"), "{}", scan_line(2, 0));
    }

    #[test]
    fn summary_line_reports_total_elapsed_and_average() {
        let line = summary_line(
            (88.2 * GB as f64) as u64,
            Duration::from_secs(761),
            Some(118.0 * MB as f64),
        );
        assert!(line.contains("88.2G"), "got {line:?}");
        assert!(line.contains("12m41s"), "got {line:?}");
        assert!(line.contains("118MB/s"), "got {line:?}");
    }
}
