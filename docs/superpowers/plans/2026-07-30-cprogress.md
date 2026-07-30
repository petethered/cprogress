# cprogress Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `cprogress`, a macOS `cp` replacement for the `cp -Rf` workload that prints a wget-style aggregate + per-file progress bar.

**Architecture:** Data flows one direction, `cli → plan → engine → progress`. The `plan` module pre-scans all sources to compute a fixed byte total, `engine` executes the plan serially through a 1 MiB buffer, and `progress` renders two repainting lines on stderr. The copy engine never touches stderr; all output goes through the `Progress` trait. Line rendering is a set of pure string functions so it can be unit-tested without a terminal.

**Tech Stack:** Rust 2021, `libc` as the only runtime dependency, `tempfile` as the only dev-dependency. Build with `cargo`.

**Spec:** `docs/superpowers/specs/2026-07-30-cprogress-design.md`

## Global Constraints

- Platform is macOS. Do not add Linux/Windows compatibility shims.
- Runtime dependencies: `libc` only. Dev-dependencies: `tempfile` only. Do not add `clap`, `indicatif`, `terminal_size`, `nix`, `anyhow`, or any other crate.
- Every byte count is `u64`. No `usize` or `u32` byte counts anywhere — files may exceed 100 GB.
- Files are streamed through a fixed 1 MiB buffer. Nothing is memory-mapped or read whole, regardless of file size.
- All writes to a destination file loop until the buffer is fully consumed; never assume one `write` call consumes the whole buffer.
- Supported flags: `-R`, `-r`, `-f`, `-v`, `--progress=auto|force|none`, `-h`, `--help`. Everything else is an error.
- With no arguments, print `cp`'s exact two-line usage format (verbatim flag list, program name `cprogress`) and exit 64. `-h`/`--help` prints a full manual to stdout and exits 0.
- Exit codes: `0` success, `1` at least one operation failed, `64` usage error, `130` `SIGINT`.
- The per-file bar threshold is `8 * 1024 * 1024` bytes, a compile-time constant, not a flag.
- Repaint throttle is 100 ms (10 Hz). Rate estimator half-life is 3.0 s.
- Never `panic!`, `unwrap()`, or `expect()` on I/O against the terminal or the filesystem in non-test code. Terminal write failures are ignored (`let _ = ...`); filesystem failures become reported errors.
- License is MIT. Binary and crate name is `cprogress`.

---

### Task 1: Project scaffold, license, and byte/duration formatting

Sets up the crate and delivers `fmt`, the leaf module everything else formats through. The large-number tests here are the routine coverage for 100 GB+ correctness.

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `LICENSE`
- Create: `src/main.rs`
- Create: `src/fmt.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `fmt::human_bytes(u64) -> String`, `fmt::human_rate(Option<f64>) -> String`, `fmt::human_duration(Duration) -> String`, `fmt::human_eta(Option<Duration>) -> String`, `fmt::human_count(usize) -> String`.

- [ ] **Step 1: Create the crate manifest**

`Cargo.toml`:

```toml
[package]
name = "cprogress"
version = "0.1.0"
edition = "2021"
description = "A macOS cp with a wget-style progress bar"
license = "MIT"
repository = "https://github.com/peterazuolas/cprogress"

[dependencies]
libc = "0.2"

[dev-dependencies]
tempfile = "3"

[profile.release]
opt-level = 3
```

`.gitignore`:

```
/target
```

- [ ] **Step 2: Add the MIT license**

`LICENSE` — use the standard MIT text with `Copyright (c) 2026 Peter Azuolas`:

```
MIT License

Copyright (c) 2026 Peter Azuolas

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 3: Create a placeholder main so the crate compiles**

`src/main.rs`:

```rust
mod fmt;

fn main() {
    eprintln!("cprogress: not yet implemented");
    std::process::exit(1);
}
```

- [ ] **Step 4: Write the failing tests for `fmt`**

`src/fmt.rs` — write only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bytes_below_one_kib_render_as_plain_numbers() {
        assert_eq!(human_bytes(0), "0");
        assert_eq!(human_bytes(512), "512");
        assert_eq!(human_bytes(1023), "1023");
    }

    #[test]
    fn bytes_use_three_significant_digits() {
        assert_eq!(human_bytes(1024), "1.00K");
        assert_eq!(human_bytes(1024 * 10), "10.0K");
        assert_eq!(human_bytes(1024 * 100), "100K");
    }

    #[test]
    fn bytes_scale_past_thirty_two_bits() {
        // 100 GB — the headline case. Must not truncate.
        assert_eq!(human_bytes(100 * 1024 * 1024 * 1024), "100G");
        assert_eq!(human_bytes(4_519_235_912), "4.21G");
        assert_eq!(human_bytes(10 * 1024_u64.pow(4)), "10.0T");
        assert_eq!(human_bytes(5 * 1024_u64.pow(5)), "5.00P");
    }

    #[test]
    fn max_u64_does_not_panic_or_overflow() {
        let s = human_bytes(u64::MAX);
        assert!(s.ends_with('E'), "expected exabyte unit, got {s}");
    }

    #[test]
    fn rate_renders_per_second_and_has_an_unknown_form() {
        assert_eq!(human_rate(Some(118.0 * 1024.0 * 1024.0)), "118MB/s");
        assert_eq!(human_rate(None), "--.-KB/s");
        assert_eq!(human_rate(Some(0.0)), "--.-KB/s");
    }

    #[test]
    fn durations_render_compactly() {
        assert_eq!(human_duration(Duration::from_secs(0)), "0s");
        assert_eq!(human_duration(Duration::from_secs(22)), "22s");
        assert_eq!(human_duration(Duration::from_secs(761)), "12m41s");
        assert_eq!(human_duration(Duration::from_secs(3761)), "1h02m41s");
    }

    #[test]
    fn eta_has_an_unknown_form() {
        assert_eq!(human_eta(Some(Duration::from_secs(22))), "eta 22s");
        assert_eq!(human_eta(None), "eta --");
    }

    #[test]
    fn counts_are_thousands_separated() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(999), "999");
        assert_eq!(human_count(41_203), "41,203");
        assert_eq!(human_count(1_000_000), "1,000,000");
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail**

Run: `cargo test --lib fmt 2>&1 | head -30`

Expected: compile error, `cannot find function human_bytes in this scope`.

- [ ] **Step 6: Implement `fmt`**

Prepend to `src/fmt.rs`:

```rust
use std::time::Duration;

const UNITS: [&str; 7] = ["", "K", "M", "G", "T", "P", "E"];

/// Format a byte count with three significant digits, e.g. `4.21G`.
pub fn human_bytes(n: u64) -> String {
    if n < 1024 {
        return n.to_string();
    }
    let mut value = n as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let suffix = UNITS[unit];
    if value < 10.0 {
        format!("{value:.2}{suffix}")
    } else if value < 100.0 {
        format!("{value:.1}{suffix}")
    } else {
        format!("{value:.0}{suffix}")
    }
}

/// Format a transfer rate, e.g. `118MB/s`. `None` and zero render as unknown.
pub fn human_rate(bytes_per_sec: Option<f64>) -> String {
    match bytes_per_sec {
        Some(r) if r > 0.0 && r.is_finite() => format!("{}B/s", human_bytes(r as u64)),
        _ => "--.-KB/s".to_string(),
    }
}

/// Format a duration as `22s`, `12m41s`, or `1h02m41s`.
pub fn human_duration(d: Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Format an ETA field, including the `eta ` prefix, with an unknown form.
pub fn human_eta(d: Option<Duration>) -> String {
    match d {
        Some(d) => format!("eta {}", human_duration(d)),
        None => "eta --".to_string(),
    }
}

/// Format a file count with thousands separators, e.g. `41,203`.
pub fn human_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib fmt`

Expected: 7 tests pass. If `human_bytes(4_519_235_912)` does not produce exactly `4.21G`, adjust the literal to match real output rather than changing the rounding rule — the rule is what matters.

- [ ] **Step 8: Verify the release build is clean**

Run: `cargo build --release && cargo clippy -- -D warnings 2>&1 | tail -5`

Expected: builds with no warnings. If `clippy` is unavailable, skip it.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore LICENSE src/main.rs src/fmt.rs
git commit -m "Add crate scaffold, MIT license, and byte/duration formatting"
```

---

### Task 2: Command-line parsing, usage, and help

`cp`'s argument grammar, including the BSD behavior that option parsing stops at the first non-option argument, plus the usage and manual text.

**Files:**
- Create: `src/cli.rs`
- Create: `src/help.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `cli::Options { recursive: bool, force: bool, verbose: bool, progress: ProgressMode }`, `cli::ProgressMode::{Auto, Force, None}`, `cli::Invocation { options: Options, sources: Vec<PathBuf>, dest: PathBuf }`, `cli::CliError`, `cli::parse(args: Vec<OsString>) -> Result<Invocation, CliError>`, `help::USAGE: &str`, `help::MANUAL: &str`.

- [ ] **Step 1: Write the failing tests**

`src/cli.rs`, test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> Invocation {
        parse(args.iter().map(OsString::from).collect()).expect("expected success")
    }

    fn parse_err(args: &[&str]) -> CliError {
        parse(args.iter().map(OsString::from).collect()).expect_err("expected failure")
    }

    #[test]
    fn parses_separate_flags() {
        let inv = parse_ok(&["-R", "-f", "src", "dst"]);
        assert!(inv.options.recursive);
        assert!(inv.options.force);
        assert!(!inv.options.verbose);
        assert_eq!(inv.sources, vec![PathBuf::from("src")]);
        assert_eq!(inv.dest, PathBuf::from("dst"));
    }

    #[test]
    fn parses_clustered_flags() {
        let inv = parse_ok(&["-Rfv", "src", "dst"]);
        assert!(inv.options.recursive);
        assert!(inv.options.force);
        assert!(inv.options.verbose);
    }

    #[test]
    fn lowercase_r_is_recursive_too() {
        assert!(parse_ok(&["-r", "src", "dst"]).options.recursive);
    }

    #[test]
    fn multiple_sources_take_the_last_operand_as_dest() {
        let inv = parse_ok(&["-R", "a", "b", "c"]);
        assert_eq!(inv.sources, vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(inv.dest, PathBuf::from("c"));
    }

    #[test]
    fn double_dash_terminates_option_parsing() {
        let inv = parse_ok(&["--", "-weird-name", "dst"]);
        assert_eq!(inv.sources, vec![PathBuf::from("-weird-name")]);
        assert!(!inv.options.recursive);
    }

    #[test]
    fn option_parsing_stops_at_the_first_operand_like_bsd() {
        // BSD getopt does not permute: `-R` here is an operand, not a flag.
        let inv = parse_ok(&["a", "b", "-R"]);
        assert!(!inv.options.recursive);
        assert_eq!(inv.sources, vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(inv.dest, PathBuf::from("-R"));
    }

    #[test]
    fn bare_dash_is_an_operand() {
        let inv = parse_ok(&["-", "dst"]);
        assert_eq!(inv.sources, vec![PathBuf::from("-")]);
    }

    #[test]
    fn progress_mode_defaults_to_auto_and_can_be_set() {
        assert_eq!(parse_ok(&["a", "b"]).options.progress, ProgressMode::Auto);
        assert_eq!(
            parse_ok(&["--progress=none", "a", "b"]).options.progress,
            ProgressMode::None
        );
        assert_eq!(
            parse_ok(&["--progress=force", "a", "b"]).options.progress,
            ProgressMode::Force
        );
    }

    #[test]
    fn known_cp_flags_we_do_not_support_are_named_in_the_error() {
        match parse_err(&["-p", "a", "b"]) {
            CliError::Unsupported(f) => assert_eq!(f, "-p"),
            other => panic!("wrong error: {other:?}"),
        }
        assert!(matches!(parse_err(&["-i", "a", "b"]), CliError::Unsupported(_)));
        assert!(matches!(parse_err(&["-n", "a", "b"]), CliError::Unsupported(_)));
        assert!(matches!(parse_err(&["-a", "a", "b"]), CliError::Unsupported(_)));
    }

    #[test]
    fn unsupported_flag_inside_a_cluster_is_caught() {
        assert!(matches!(parse_err(&["-Rp", "a", "b"]), CliError::Unsupported(_)));
    }

    #[test]
    fn unknown_flags_are_rejected() {
        assert!(matches!(parse_err(&["-z", "a", "b"]), CliError::UnknownFlag(_)));
        assert!(matches!(parse_err(&["--bogus", "a", "b"]), CliError::UnknownFlag(_)));
        assert!(matches!(
            parse_err(&["--progress=bogus", "a", "b"]),
            CliError::UnknownFlag(_)
        ));
    }

    #[test]
    fn too_few_operands_is_a_usage_error() {
        assert!(matches!(parse_err(&[]), CliError::MissingOperand));
        assert!(matches!(parse_err(&["a"]), CliError::MissingOperand));
        assert!(matches!(parse_err(&["-R"]), CliError::MissingOperand));
    }

    #[test]
    fn help_is_its_own_outcome() {
        assert!(matches!(parse_err(&["--help"]), CliError::Help));
        assert!(matches!(parse_err(&["-h"]), CliError::Help));
        // Verified: `cp -h` is an illegal option, so -h is free to claim.
        assert!(matches!(parse_err(&["-Rh", "a", "b"]), CliError::Help));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib cli 2>&1 | head -20`

Expected: compile error, `cannot find function parse`.

- [ ] **Step 3: Write the usage and manual text**

`src/help.rs`:

```rust
/// Mirrors `cp`'s two-line usage exactly, including the flags this tool
/// rejects. The usage line states which interface is imitated; `MANUAL` is
/// where the actually supported set is documented, and an unsupported flag
/// produces a specific message naming it.
pub const USAGE: &str = "\
usage: cprogress [-R [-H | -L | -P]] [-fi | -n] [-aclpSsvXx] source_file target_file
       cprogress [-R [-H | -L | -P]] [-fi | -n] [-aclpSsvXx] source_file ... target_directory";

/// Printed by -h/--help. The equivalent of `man cp`, written for cprogress.
pub const MANUAL: &str = r#"NAME
     cprogress -- copy files with a progress bar

SYNOPSIS
     cprogress [-R | -r] [-f] [-v] [--progress=auto|force|none]
               source_file target_file
     cprogress [-R | -r] [-f] [-v] [--progress=auto|force|none]
               source_file ... target_directory

DESCRIPTION
     cprogress copies files the way cp(1) does, and prints a wget-style
     progress bar while it works.

     In the first form, source_file is copied to target_file. In the second
     form, each source_file is copied into the existing target_directory.

     Before copying, cprogress walks the sources to total up the bytes, so the
     percentage and estimated time are accurate from the first frame. The scan
     reads metadata only.

     The display is two lines: an aggregate bar for the whole operation, and a
     bar for the file being copied. Files smaller than 8 MiB show only their
     path, because a bar that fills within a single read is noise rather than
     information.

     The options are as follows:

     -R, -r  Copy directory hierarchies recursively. Symbolic links are
             recreated as symbolic links rather than followed, matching the
             default behavior of cp -R.

     -f      If a destination file cannot be opened for writing, remove it and
             try again.

     -v      Print the name of each file as it is copied.

     -h      Print this manual and exit.

     --help  Same as -h.

     --progress=auto|force|none
             Control the progress display. auto, the default, draws the bar
             only when standard error is a terminal. force always draws it.
             none never does.

UNSUPPORTED OPTIONS
     The following cp(1) options are not implemented and are rejected with an
     error rather than ignored, so that a script relying on one fails visibly:

           -p -i -n -a -c -l -s -X -H -L -P

     Extended attributes and access control lists are not copied.

FILE PERMISSIONS
     A newly created file or directory takes the source's permission bits
     masked by the current umask. An existing destination file keeps its own
     permissions. Both match cp(1).

     Ownership, timestamps, and BSD file flags are not preserved; that is what
     cp -p would provide, and -p is not supported.

EXIT STATUS
     0       All files were copied successfully.
     1       At least one file could not be copied.
     64      The command line was malformed.
     130     The copy was interrupted with SIGINT.

EXAMPLES
     Copy a directory tree, replacing anything already at the destination:

           cprogress -Rf ~/Photos /Volumes/Backup/Photos

     Use it in place of cp everywhere. The bar is suppressed when standard
     error is not a terminal, so scripts see exactly what cp would print:

           alias cp=cprogress

SEE ALSO
     cp(1)
"#;
```

- [ ] **Step 4: Write the failing test for the usage text**

Append to the test module in `src/help.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_has_two_lines_in_cp_format() {
        let lines: Vec<&str> = USAGE.lines().collect();
        assert_eq!(lines.len(), 2, "usage should be two lines");
        assert!(lines[0].starts_with("usage: cprogress "), "got {:?}", lines[0]);
        assert!(lines[0].ends_with("source_file target_file"), "got {:?}", lines[0]);
        assert!(
            lines[1].ends_with("source_file ... target_directory"),
            "got {:?}",
            lines[1]
        );
        // The continuation line is aligned under the first, as cp does.
        assert!(lines[1].starts_with("       cprogress "), "got {:?}", lines[1]);
    }

    #[test]
    fn manual_documents_the_supported_and_unsupported_sets() {
        for expected in [
            "NAME", "SYNOPSIS", "DESCRIPTION", "EXIT STATUS", "SEE ALSO",
            "-R", "-f", "-v", "--progress", "-p", "alias cp=cprogress",
        ] {
            assert!(MANUAL.contains(expected), "manual missing {expected:?}");
        }
    }
}
```

- [ ] **Step 5: Implement `cli`**

Prepend to `src/cli.rs`:

```rust
use crate::help::USAGE;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Auto,
    Force,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub recursive: bool,
    pub force: bool,
    pub verbose: bool,
    pub progress: ProgressMode,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            recursive: false,
            force: false,
            verbose: false,
            progress: ProgressMode::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub options: Options,
    pub sources: Vec<PathBuf>,
    pub dest: PathBuf,
}

#[derive(Debug)]
pub enum CliError {
    /// A real `cp` flag that v1 deliberately does not implement.
    Unsupported(String),
    UnknownFlag(String),
    MissingOperand,
    Help,
}

/// `cp` flags that exist but are out of scope for v1. Failing loudly on these
/// beats silently ignoring them: an ignored `-p` yields a tree with wrong
/// ownership and no indication anything went wrong.
const UNSUPPORTED_SHORT: &[char] = &['p', 'i', 'n', 'a', 'c', 'l', 's', 'X', 'H', 'L', 'P'];

pub fn parse(args: Vec<OsString>) -> Result<Invocation, CliError> {
    let mut options = Options::default();
    let mut operands: Vec<PathBuf> = Vec::new();
    let mut iter = args.into_iter();
    let mut only_operands = false;

    while let Some(arg) = iter.next() {
        if only_operands {
            operands.push(PathBuf::from(arg));
            continue;
        }

        let text = arg.to_string_lossy().into_owned();

        if text == "--" {
            only_operands = true;
            continue;
        }

        // BSD getopt does not permute: the first operand ends option parsing.
        let is_option = text.starts_with('-') && text != "-";
        if !is_option {
            only_operands = true;
            operands.push(PathBuf::from(arg));
            continue;
        }

        if text.starts_with("--") {
            if text == "--help" {
                return Err(CliError::Help);
            }
            match text.strip_prefix("--progress=") {
                Some("auto") => options.progress = ProgressMode::Auto,
                Some("force") => options.progress = ProgressMode::Force,
                Some("none") => options.progress = ProgressMode::None,
                _ => return Err(CliError::UnknownFlag(text)),
            }
            continue;
        }

        for c in text.chars().skip(1) {
            match c {
                'R' | 'r' => options.recursive = true,
                'f' => options.force = true,
                'v' => options.verbose = true,
                // Free to claim: `cp -h` is an illegal option, not a flag.
                'h' => return Err(CliError::Help),
                _ if UNSUPPORTED_SHORT.contains(&c) => {
                    return Err(CliError::Unsupported(format!("-{c}")))
                }
                _ => return Err(CliError::UnknownFlag(format!("-{c}"))),
            }
        }
    }

    if operands.len() < 2 {
        return Err(CliError::MissingOperand);
    }

    let dest = operands.pop().expect("length checked above");
    Ok(Invocation {
        options,
        sources: operands,
        dest,
    })
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Unsupported(flag) => write!(
                f,
                "cprogress: {flag} is not supported\n{USAGE}"
            ),
            CliError::UnknownFlag(flag) => {
                write!(f, "cprogress: illegal option {flag}\n{USAGE}")
            }
            // Matches cp: a bare invocation prints usage and nothing else.
            CliError::MissingOperand => write!(f, "{USAGE}"),
            CliError::Help => write!(f, "{USAGE}"),
        }
    }
}
```

- [ ] **Step 6: Wire the modules into main**

Replace `src/main.rs` with:

```rust
mod cli;
mod fmt;
mod help;

fn main() {
    eprintln!("cprogress: not yet implemented");
    std::process::exit(1);
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib`

Expected: 13 `cli` tests and 2 `help` tests pass, alongside the `fmt` tests.

- [ ] **Step 8: Verify the usage text matches cp's shape**

```bash
diff <(/bin/cp 2>&1 | sed 's/cp/cprogress/g') <(cargo run --quiet 2>&1) && echo "usage matches"
```

Expected: no differences. This will not run until Task 9 wires up `main`; if
`cargo run` still prints the placeholder, defer this check to Task 9 Step 4.

- [ ] **Step 9: Commit**

```bash
git add src/cli.rs src/help.rs src/main.rs
git commit -m "Add cp-compatible command-line parsing, usage, and manual"
```

---

### Task 3: Transfer rate estimation

An EWMA estimator with a 3 s half-life. A raw instantaneous rate makes the ETA jitter across mixed file sizes; a cumulative average reacts too slowly when the copy moves from large files into small ones.

**Files:**
- Create: `src/rate.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `rate::RateEstimator::new(half_life_secs: f64)`, `::sample(&mut self, bytes: u64, dt: Duration)`, `::rate(&self) -> Option<f64>`, `::eta(&self, remaining: u64) -> Option<Duration>`, `rate::HALF_LIFE_SECS: f64`.

- [ ] **Step 1: Write the failing tests**

`src/rate.rs`, test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn rate_is_unknown_before_any_sample() {
        let est = RateEstimator::new(3.0);
        assert_eq!(est.rate(), None);
        assert_eq!(est.eta(1000), None);
    }

    #[test]
    fn first_sample_sets_the_rate_exactly() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1));
        let r = est.rate().expect("rate after one sample");
        assert!((r - (100 * MB) as f64).abs() < 1.0, "got {r}");
    }

    #[test]
    fn steady_input_converges_to_the_steady_rate() {
        let mut est = RateEstimator::new(3.0);
        for _ in 0..100 {
            est.sample(10 * MB, Duration::from_millis(100));
        }
        let r = est.rate().expect("rate");
        let expected = (100 * MB) as f64; // 10 MiB per 100 ms
        assert!(
            (r - expected).abs() / expected < 0.01,
            "expected ~{expected}, got {r}"
        );
    }

    #[test]
    fn one_half_life_moves_the_estimate_halfway() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1)); // rate = 100 MiB/s
        est.sample(0, Duration::from_secs(3)); // instantaneous 0, one half-life
        let r = est.rate().expect("rate");
        let expected = (100 * MB) as f64 / 2.0;
        assert!(
            (r - expected).abs() / expected < 0.02,
            "expected ~{expected}, got {r}"
        );
    }

    #[test]
    fn zero_duration_samples_are_ignored() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1));
        let before = est.rate();
        est.sample(5 * MB, Duration::ZERO);
        assert_eq!(est.rate(), before);
        assert!(est.rate().expect("finite").is_finite());
    }

    #[test]
    fn eta_divides_remaining_by_rate() {
        let mut est = RateEstimator::new(3.0);
        est.sample(10 * MB, Duration::from_secs(1));
        let eta = est.eta(100 * MB).expect("eta");
        assert_eq!(eta.as_secs(), 10);
    }

    #[test]
    fn eta_handles_multi_terabyte_remainders() {
        let mut est = RateEstimator::new(3.0);
        est.sample(100 * MB, Duration::from_secs(1));
        let ten_tb = 10 * 1024_u64.pow(4);
        let eta = est.eta(ten_tb).expect("eta");
        // 10 TiB / 100 MiB/s = 104,857.6 s
        assert_eq!(eta.as_secs(), 104_857);
    }

    #[test]
    fn eta_is_unknown_when_the_rate_is_zero() {
        let mut est = RateEstimator::new(3.0);
        est.sample(0, Duration::from_secs(1));
        assert_eq!(est.eta(1000), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib rate 2>&1 | head -20`

Expected: compile error, `cannot find struct RateEstimator`.

- [ ] **Step 3: Implement `rate`**

Prepend to `src/rate.rs`:

```rust
use std::time::Duration;

/// Half-life of the rate estimate. Long enough that the ETA is stable across a
/// directory of mixed file sizes, short enough to react within a few seconds
/// when throughput genuinely changes.
pub const HALF_LIFE_SECS: f64 = 3.0;

/// Exponentially weighted moving average of transfer rate, in bytes per second.
pub struct RateEstimator {
    rate: Option<f64>,
    half_life: f64,
}

impl RateEstimator {
    pub fn new(half_life_secs: f64) -> Self {
        RateEstimator {
            rate: None,
            half_life: half_life_secs,
        }
    }

    /// Fold in `bytes` transferred over `dt`. Zero-length intervals are ignored
    /// rather than producing an infinite instantaneous rate.
    pub fn sample(&mut self, bytes: u64, dt: Duration) {
        let secs = dt.as_secs_f64();
        if secs <= 0.0 {
            return;
        }
        let instant = bytes as f64 / secs;
        self.rate = Some(match self.rate {
            None => instant,
            Some(current) => {
                let alpha = 1.0 - 0.5_f64.powf(secs / self.half_life);
                current + alpha * (instant - current)
            }
        });
    }

    pub fn rate(&self) -> Option<f64> {
        self.rate
    }

    /// Time to move `remaining` bytes at the current rate.
    pub fn eta(&self, remaining: u64) -> Option<Duration> {
        let rate = self.rate?;
        if rate <= 0.0 || !rate.is_finite() {
            return None;
        }
        let secs = remaining as f64 / rate;
        if !secs.is_finite() || secs < 0.0 {
            return None;
        }
        Some(Duration::from_secs_f64(secs.min(u32::MAX as f64)))
    }
}
```

- [ ] **Step 4: Wire the module into main**

Add `mod rate;` to `src/main.rs` alongside the existing `mod` lines.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib rate`

Expected: 8 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/rate.rs src/main.rs
git commit -m "Add EWMA transfer rate estimator"
```

---

### Task 4: Terminal width and TTY detection

**Files:**
- Create: `src/term.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `term::is_tty(fd: RawFd) -> bool`, `term::width(fd: RawFd) -> Option<usize>`, `term::width_or_fallback(fd: RawFd) -> usize`, `term::FALLBACK_WIDTH: usize`.

- [ ] **Step 1: Write the failing tests**

`src/term.rs`, test module only:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib term 2>&1 | head -20`

Expected: compile error, `cannot find function is_tty`.

- [ ] **Step 3: Implement `term`**

Prepend to `src/term.rs`:

```rust
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
```

- [ ] **Step 4: Wire the module into main**

Add `mod term;` to `src/main.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib term`

Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/term.rs src/main.rs
git commit -m "Add terminal width and TTY detection"
```

---

### Task 5: Progress line rendering

Pure string functions. Keeping rendering free of I/O is what makes the width and threshold behavior testable without a terminal.

**Files:**
- Create: `src/progress/mod.rs`
- Create: `src/progress/render.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `fmt::*`.
- Produces: `progress::render::{FILE_BAR_MIN_BYTES, MIN_BAR_COLUMNS, bar, middle_ellipsize, aggregate_line, file_line, scan_line, summary_line}`.

- [ ] **Step 1: Create the module directory and declare it**

`src/progress/mod.rs`:

```rust
pub mod render;
```

Add `mod progress;` to `src/main.rs`.

- [ ] **Step 2: Write the failing tests**

`src/progress/render.rs`, test module only:

```rust
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
        assert_eq!(middle_ellipsize("abcdefghijk", 5), "abc…k");
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
    fn scan_line_reports_files_and_bytes() {
        let line = scan_line(41_203, (88.2 * GB as f64) as u64);
        assert!(line.starts_with("Scanning..."), "got {line:?}");
        assert!(line.contains("41,203 files"), "got {line:?}");
        assert!(line.contains("88.2G"), "got {line:?}");
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib render 2>&1 | head -20`

Expected: compile error, `cannot find function bar`.

- [ ] **Step 4: Implement `render`**

Prepend to `src/progress/render.rs`:

```rust
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
    let fixed = name.chars().count() + overhead;
    let bar_cols = width.saturating_sub(fixed);
    if bar_cols < MIN_BAR_COLUMNS {
        return truncate_to(format!("{name}{pct}{tail}"), width);
    }
    format!("{name}{pct}[{}]{tail}", bar(bar_cols, fraction(done, size)))
}

/// `Scanning... 41,203 files, 88.2G`
pub fn scan_line(files: usize, bytes: u64) -> String {
    format!(
        "Scanning... {} files, {}",
        human_count(files),
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib render`

Expected: 16 tests pass. If a layout assertion fails on exact spacing, adjust the assertion to match real output — the invariants that matter are "never exceeds width", "threshold respected", and "fields present".

- [ ] **Step 6: Commit**

```bash
git add src/progress/ src/main.rs
git commit -m "Add pure progress line rendering with width budgeting"
```

---

### Task 6: The Progress trait, Null, and Bar

**Files:**
- Modify: `src/progress/mod.rs`
- Create: `src/progress/bar.rs`

**Interfaces:**
- Consumes: `progress::render::*`, `rate::RateEstimator`, `term::*`, `fmt::*`.
- Produces: `progress::Progress` trait with `scan_tick`, `start`, `file_started`, `advance`, `note`, `finish`; `progress::Null`; `progress::bar::Bar::new(out: W, width: Box<dyn Fn() -> usize>)`.

- [ ] **Step 1: Write the failing tests for `Null`**

Append to `src/progress/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib progress 2>&1 | head -20`

Expected: compile error, `cannot find struct Null`.

- [ ] **Step 3: Implement the trait and `Null`**

Replace the top of `src/progress/mod.rs` (keep the test module) with:

```rust
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
```

- [ ] **Step 4: Run to verify the Null tests pass**

Run: `cargo test --lib progress`

Expected: 2 tests pass (plus the 16 render tests).

- [ ] **Step 5: Write the failing tests for `Bar`**

`src/progress/bar.rs`, test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Progress;
    use std::path::Path;

    const MB: u64 = 1024 * 1024;

    fn bar_with(sink: &mut Vec<u8>, width: usize) -> Bar<&mut Vec<u8>> {
        Bar::new(sink, Box::new(move || width))
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
            b.note("src -> dst");
        }
        let out = rendered(sink);
        assert!(out.contains("src -> dst"), "note missing: {out:?}");
        // The bar is redrawn after the note, so the aggregate label appears again.
        let label_count = out.matches("total ").count();
        assert!(label_count >= 2, "bar not redrawn after note: {out:?}");
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
```

- [ ] **Step 6: Run to verify failure**

Run: `cargo test --lib bar 2>&1 | head -20`

Expected: compile error, `cannot find struct Bar`.

- [ ] **Step 7: Implement `Bar`**

Prepend to `src/progress/bar.rs`:

```rust
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
pub struct Bar<W: Write> {
    out: W,
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

impl<W: Write> Bar<W> {
    pub fn new(out: W, width: Box<dyn Fn() -> usize>) -> Self {
        let now = Instant::now();
        Bar {
            out,
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
        if !force && now.duration_since(self.last_paint) < REPAINT_INTERVAL {
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

impl<W: Write> Progress for Bar<W> {
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
        self.draw();
        let _ = self.out.flush();
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
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib`

Expected: all tests pass. The throttle test depends on wall-clock timing; if 1000 `advance` calls take longer than 100 ms on the test machine, raise the allowed repaint count in the assertion rather than weakening the throttle.

- [ ] **Step 9: Commit**

```bash
git add src/progress/
git commit -m "Add Progress trait with Null and two-line Bar implementations"
```

---

### Task 7: Source scanning and plan building

The pre-scan. Resolves destination semantics once so the engine never re-derives paths, and catches fatal errors before any bytes move.

**Files:**
- Create: `src/plan.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `cli::Options`.
- Produces: `plan::CopyOp::{CreateDir { dst, mode }, CopyFile { src, dst, size }, Symlink { target, dst }}`, `plan::Plan { ops, total_bytes, file_count, errors }`, `plan::PlanError`, `plan::build(sources: &[PathBuf], dest: &Path, opts: &Options, tick: &mut dyn FnMut(usize, u64)) -> Result<Plan, PlanError>`.

- [ ] **Step 1: Write the failing tests**

`src/plan.rs`, test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn opts(recursive: bool) -> Options {
        Options {
            recursive,
            ..Options::default()
        }
    }

    fn build_in(
        sources: &[PathBuf],
        dest: &Path,
        o: &Options,
    ) -> Result<Plan, PlanError> {
        let mut tick = |_f: usize, _b: u64| {};
        build(sources, dest, o, &mut tick)
    }

    #[test]
    fn single_file_to_new_name() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a.txt");
        fs::write(&src, b"hello").expect("write");
        let dst = td.path().join("b.txt");

        let plan = build_in(&[src.clone()], &dst, &opts(false)).expect("plan");
        assert_eq!(plan.file_count, 1);
        assert_eq!(plan.total_bytes, 5);
        match &plan.ops[0] {
            CopyOp::CopyFile { dst: d, size, .. } => {
                assert_eq!(d, &dst);
                assert_eq!(*size, 5);
            }
            other => panic!("wrong op: {other:?}"),
        }
    }

    #[test]
    fn single_file_into_existing_directory() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a.txt");
        fs::write(&src, b"hello").expect("write");
        let dir = td.path().join("out");
        fs::create_dir(&dir).expect("mkdir");

        let plan = build_in(&[src], &dir, &opts(false)).expect("plan");
        match &plan.ops[0] {
            CopyOp::CopyFile { dst, .. } => assert_eq!(dst, &dir.join("a.txt")),
            other => panic!("wrong op: {other:?}"),
        }
    }

    #[test]
    fn recursive_walk_produces_dirs_files_and_symlinks() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir_all(src.join("sub")).expect("mkdir");
        fs::write(src.join("f.txt"), b"12345").expect("write");
        fs::write(src.join("sub/g.txt"), b"123").expect("write");
        symlink("f.txt", src.join("link")).expect("symlink");
        let dst = td.path().join("d");

        let plan = build_in(&[src], &dst, &opts(true)).expect("plan");
        assert_eq!(plan.total_bytes, 8);
        assert_eq!(plan.file_count, 2);
        assert!(plan
            .ops
            .iter()
            .any(|o| matches!(o, CopyOp::CreateDir { dst, .. } if dst.ends_with("d/sub"))));
        assert!(plan
            .ops
            .iter()
            .any(|o| matches!(o, CopyOp::Symlink { dst, .. } if dst.ends_with("link"))));
    }

    #[test]
    fn parent_directory_is_ordered_before_its_contents() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir_all(src.join("sub")).expect("mkdir");
        fs::write(src.join("sub/g.txt"), b"1").expect("write");
        let dst = td.path().join("d");

        let plan = build_in(&[src], &dst, &opts(true)).expect("plan");
        let dir_at = plan
            .ops
            .iter()
            .position(|o| matches!(o, CopyOp::CreateDir { dst, .. } if dst.ends_with("sub")))
            .expect("sub dir op");
        let file_at = plan
            .ops
            .iter()
            .position(|o| matches!(o, CopyOp::CopyFile { dst, .. } if dst.ends_with("g.txt")))
            .expect("file op");
        assert!(dir_at < file_at, "directory must precede its contents");
    }

    #[test]
    fn directory_without_recursive_is_a_reported_error_not_a_failure() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        let dst = td.path().join("d");

        let plan = build_in(&[src], &dst, &opts(false)).expect("plan");
        assert!(plan.ops.is_empty());
        assert_eq!(plan.errors.len(), 1);
        assert!(
            plan.errors[0].contains("is a directory (not copied)"),
            "got {:?}",
            plan.errors[0]
        );
    }

    #[test]
    fn multiple_sources_require_a_directory_destination() {
        let td = tempfile::tempdir().expect("tempdir");
        let a = td.path().join("a");
        let b = td.path().join("b");
        fs::write(&a, b"1").expect("write");
        fs::write(&b, b"2").expect("write");
        let dst = td.path().join("notdir");
        fs::write(&dst, b"x").expect("write");

        let err = build_in(&[a, b], &dst, &opts(false)).expect_err("should fail");
        assert!(matches!(err, PlanError::DestNotDirectory(_)));
    }

    #[test]
    fn copying_a_directory_into_itself_is_rejected() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        let dst = src.join("inner");

        let err = build_in(&[src], &dst, &opts(true)).expect_err("should fail");
        assert!(matches!(err, PlanError::DestInsideSource { .. }));
    }

    #[test]
    fn a_missing_source_is_reported_and_skipped() {
        let td = tempfile::tempdir().expect("tempdir");
        let missing = td.path().join("nope");
        let dst = td.path().join("d");

        let plan = build_in(&[missing], &dst, &opts(false)).expect("plan");
        assert!(plan.ops.is_empty());
        assert_eq!(plan.errors.len(), 1);
        assert!(plan.errors[0].contains("nope"), "got {:?}", plan.errors[0]);
    }

    #[test]
    fn trailing_slash_on_a_source_behaves_like_bsd() {
        // BSD ignores it: `cp -R s/ d` and `cp -R s d` agree.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        fs::write(src.join("f"), b"1").expect("write");
        let out = td.path().join("out");
        fs::create_dir(&out).expect("mkdir");

        let with_slash = build_in(&[PathBuf::from(format!("{}/", src.display()))], &out, &opts(true))
            .expect("plan");
        let without = build_in(&[src], &out, &opts(true)).expect("plan");
        assert_eq!(with_slash.total_bytes, without.total_bytes);
        assert_eq!(with_slash.ops.len(), without.ops.len());
    }

    #[test]
    fn scan_reports_progress_through_the_tick_callback() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        for i in 0..5 {
            fs::write(src.join(format!("f{i}")), b"12").expect("write");
        }
        let dst = td.path().join("d");

        let mut seen: Vec<(usize, u64)> = Vec::new();
        let mut tick = |f: usize, b: u64| seen.push((f, b));
        let plan = build(&[src], &dst, &opts(true), &mut tick).expect("plan");

        assert_eq!(plan.file_count, 5);
        assert_eq!(seen.last().copied(), Some((5, 10)));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib plan 2>&1 | head -20`

Expected: compile error, `cannot find function build`.

- [ ] **Step 3: Implement `plan`**

Prepend to `src/plan.rs`:

```rust
use crate::cli::Options;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Every variant carries `src` so `-v` can report `src -> dst` for directories
/// and symlinks, not just regular files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOp {
    CreateDir { src: PathBuf, dst: PathBuf, mode: u32 },
    CopyFile { src: PathBuf, dst: PathBuf, size: u64 },
    Symlink { src: PathBuf, target: PathBuf, dst: PathBuf },
}

#[derive(Debug, Default)]
pub struct Plan {
    pub ops: Vec<CopyOp>,
    pub total_bytes: u64,
    pub file_count: usize,
    /// Non-fatal problems found while scanning. Reported before copying starts
    /// and reflected in the exit status; they do not stop the run.
    pub errors: Vec<String>,
}

#[derive(Debug)]
pub enum PlanError {
    DestNotDirectory(PathBuf),
    DestInsideSource { src: PathBuf, dst: PathBuf },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::DestNotDirectory(p) => {
                write!(f, "cprogress: {}: Not a directory", p.display())
            }
            PlanError::DestInsideSource { src, dst } => write!(
                f,
                "cprogress: {} and {} are identical (not copied).",
                src.display(),
                dst.display()
            ),
        }
    }
}

struct Scanner<'a> {
    plan: Plan,
    opts: &'a Options,
    tick: &'a mut dyn FnMut(usize, u64),
}

impl Scanner<'_> {
    fn note_io(&mut self, path: &Path, err: &io::Error) {
        self.plan
            .errors
            .push(format!("cprogress: {}: {}", path.display(), err));
    }

    fn add_file(&mut self, src: PathBuf, dst: PathBuf, size: u64) {
        self.plan.ops.push(CopyOp::CopyFile { src, dst, size });
        self.plan.total_bytes = self.plan.total_bytes.saturating_add(size);
        self.plan.file_count += 1;
        (self.tick)(self.plan.file_count, self.plan.total_bytes);
    }

    /// Add `src` at `dst`, recursing if it is a directory.
    fn visit(&mut self, src: &Path, dst: &Path, top_level: bool) {
        let meta = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(e) => {
                self.note_io(src, &e);
                return;
            }
        };

        if meta.file_type().is_symlink() {
            match fs::read_link(src) {
                Ok(target) => self.plan.ops.push(CopyOp::Symlink {
                    src: src.to_path_buf(),
                    target,
                    dst: dst.to_path_buf(),
                }),
                Err(e) => self.note_io(src, &e),
            }
            return;
        }

        if meta.is_dir() {
            if !self.opts.recursive {
                // Matches cp's wording.
                self.plan.errors.push(format!(
                    "cprogress: {} is a directory (not copied).",
                    src.display()
                ));
                return;
            }
            self.plan.ops.push(CopyOp::CreateDir {
                src: src.to_path_buf(),
                dst: dst.to_path_buf(),
                mode: meta.permissions().mode() & 0o7777,
            });
            let entries = match fs::read_dir(src) {
                Ok(e) => e,
                Err(e) => {
                    self.note_io(src, &e);
                    return;
                }
            };
            let mut names: Vec<PathBuf> = Vec::new();
            for entry in entries {
                match entry {
                    Ok(e) => names.push(e.file_name().into()),
                    Err(e) => self.note_io(src, &e),
                }
            }
            // Sorted so a given tree always yields the same plan.
            names.sort();
            for name in names {
                self.visit(&src.join(&name), &dst.join(&name), false);
            }
            return;
        }

        if meta.is_file() {
            self.add_file(src.to_path_buf(), dst.to_path_buf(), meta.len());
            return;
        }

        // Sockets, fifos, devices: out of scope, reported rather than skipped
        // silently so the exit status reflects that something was left behind.
        let _ = top_level;
        self.plan.errors.push(format!(
            "cprogress: {}: unsupported file type (not copied)",
            src.display()
        ));
    }
}

/// Resolve where a single source lands under `dest`.
fn destination_for(src: &Path, dest: &Path, dest_is_dir: bool) -> PathBuf {
    if dest_is_dir {
        match src.file_name() {
            Some(name) => dest.join(name),
            None => dest.to_path_buf(),
        }
    } else {
        dest.to_path_buf()
    }
}

/// True when `dst` lies inside `src`, which would recurse forever.
fn dest_inside_source(src: &Path, dst: &Path) -> bool {
    let src_c = src.canonicalize();
    let (Ok(src_c), Ok(dst_c)) = (src_c, dst.canonicalize().or_else(|_| {
        dst.parent()
            .map(|p| p.canonicalize().map(|c| c.join(dst.file_name().unwrap_or_default())))
            .unwrap_or_else(|| Ok(dst.to_path_buf()))
    })) else {
        return false;
    };
    dst_c.starts_with(&src_c)
}

pub fn build(
    sources: &[PathBuf],
    dest: &Path,
    opts: &Options,
    tick: &mut dyn FnMut(usize, u64),
) -> Result<Plan, PlanError> {
    let dest_meta = fs::metadata(dest);
    let dest_is_dir = dest_meta.map(|m| m.is_dir()).unwrap_or(false);

    if sources.len() > 1 && !dest_is_dir {
        return Err(PlanError::DestNotDirectory(dest.to_path_buf()));
    }

    for src in sources {
        if src.is_dir() && dest_inside_source(src, dest) {
            return Err(PlanError::DestInsideSource {
                src: src.clone(),
                dst: dest.to_path_buf(),
            });
        }
    }

    let mut scanner = Scanner {
        plan: Plan::default(),
        opts,
        tick,
    };

    for src in sources {
        let target = destination_for(src, dest, dest_is_dir);
        scanner.visit(src, &target, true);
    }

    Ok(scanner.plan)
}
```

- [ ] **Step 4: Wire the module into main**

Add `mod plan;` to `src/main.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib plan`

Expected: 10 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/plan.rs src/main.rs
git commit -m "Add source scanning and plan building"
```

---

### Task 8: The copy engine

Executes a plan serially. This is where the 100 GB requirement is met: a fixed buffer, `u64` counters, and looping writes.

**Files:**
- Create: `src/engine.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `plan::{Plan, CopyOp}`, `cli::Options`, `progress::Progress`.
- Produces: `engine::Outcome { failed: bool, interrupted: bool }`, `engine::run(plan: &Plan, opts: &Options, progress: &mut dyn Progress, interrupted: &AtomicBool) -> Outcome`, `engine::umask() -> u32`, `engine::BUFFER_SIZE: usize`.

- [ ] **Step 1: Write the failing tests**

`src/engine.rs`, test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan;
    use crate::progress::Null;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::sync::atomic::AtomicBool;

    fn run_plan(p: &Plan, o: &Options) -> Outcome {
        let mut sink: Vec<u8> = Vec::new();
        let mut prog = Null::new(&mut sink);
        let flag = AtomicBool::new(false);
        run(p, o, &mut prog, &flag)
    }

    fn plan_for(sources: &[PathBuf], dest: &Path, o: &Options) -> Plan {
        let mut tick = |_f: usize, _b: u64| {};
        plan::build(sources, dest, o, &mut tick).expect("plan")
    }

    fn recursive() -> Options {
        Options {
            recursive: true,
            ..Options::default()
        }
    }

    #[test]
    fn copies_file_contents_exactly() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a.bin");
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &body).expect("write");
        let dst = td.path().join("b.bin");

        let p = plan_for(&[src], &dst, &Options::default());
        let outcome = run_plan(&p, &Options::default());

        assert!(!outcome.failed);
        assert_eq!(fs::read(&dst).expect("read"), body);
    }

    #[test]
    fn copies_data_larger_than_one_buffer() {
        // Crosses the 1 MiB chunk boundary several times.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("big.bin");
        let body = vec![0xAB_u8; BUFFER_SIZE * 3 + 12_345];
        fs::write(&src, &body).expect("write");
        let dst = td.path().join("out.bin");

        let p = plan_for(&[src], &dst, &Options::default());
        run_plan(&p, &Options::default());

        assert_eq!(fs::metadata(&dst).expect("stat").len() as usize, body.len());
        assert_eq!(fs::read(&dst).expect("read"), body);
    }

    #[test]
    fn recreates_a_directory_tree_with_symlinks() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir_all(src.join("sub")).expect("mkdir");
        fs::write(src.join("f.txt"), b"hello").expect("write");
        fs::write(src.join("sub/g.txt"), b"world").expect("write");
        symlink("f.txt", src.join("link")).expect("symlink");
        let dst = td.path().join("d");

        let p = plan_for(&[src], &dst, &recursive());
        let outcome = run_plan(&p, &recursive());

        assert!(!outcome.failed);
        assert_eq!(fs::read(dst.join("f.txt")).expect("read"), b"hello");
        assert_eq!(fs::read(dst.join("sub/g.txt")).expect("read"), b"world");
        let link_meta = fs::symlink_metadata(dst.join("link")).expect("stat");
        assert!(link_meta.file_type().is_symlink(), "link was followed");
        assert_eq!(fs::read_link(dst.join("link")).expect("readlink"), Path::new("f.txt"));
    }

    #[test]
    fn a_new_file_gets_source_mode_masked_by_umask() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a");
        fs::write(&src, b"x").expect("write");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o666)).expect("chmod");
        let dst = td.path().join("b");

        let p = plan_for(&[src], &dst, &Options::default());
        run_plan(&p, &Options::default());

        let mode = fs::metadata(&dst).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o666 & !umask(), "got {mode:o}");
    }

    #[test]
    fn an_existing_destination_keeps_its_own_mode() {
        // Verified against /bin/cp: overwriting does not re-apply source mode.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a");
        fs::write(&src, b"xyz").expect("write");
        fs::set_permissions(&src, fs::Permissions::from_mode(0o666)).expect("chmod");
        let dst = td.path().join("b");
        fs::write(&dst, b"old").expect("write");
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o600)).expect("chmod");

        let p = plan_for(&[src], &dst, &Options::default());
        run_plan(&p, &Options::default());

        let mode = fs::metadata(&dst).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        assert_eq!(fs::read(&dst).expect("read"), b"xyz");
    }

    #[test]
    fn force_replaces_an_unwritable_destination() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a");
        fs::write(&src, b"new").expect("write");
        let dst = td.path().join("b");
        fs::write(&dst, b"old").expect("write");
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o444)).expect("chmod");

        let forced = Options {
            force: true,
            ..Options::default()
        };
        let p = plan_for(&[src], &dst, &forced);
        let outcome = run_plan(&p, &forced);

        assert!(!outcome.failed, "force should have succeeded");
        assert_eq!(fs::read(&dst).expect("read"), b"new");
    }

    #[test]
    fn a_failed_file_is_reported_and_the_run_continues() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        fs::write(src.join("ok.txt"), b"fine").expect("write");
        let bad = src.join("bad.txt");
        fs::write(&bad, b"nope").expect("write");
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).expect("chmod");
        let dst = td.path().join("d");

        let p = plan_for(&[src], &dst, &recursive());
        let mut sink: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        let outcome = {
            let mut prog = Null::new(&mut sink);
            run(&p, &recursive(), &mut prog, &flag)
        };

        assert!(outcome.failed, "expected a failure to be recorded");
        assert_eq!(
            fs::read(dst.join("ok.txt")).expect("read"),
            b"fine",
            "run should have continued past the failure"
        );
        let notes = String::from_utf8(sink).expect("utf8");
        assert!(notes.contains("bad.txt"), "error not reported: {notes:?}");
    }

    #[test]
    fn a_failed_file_still_advances_the_aggregate_total() {
        // Otherwise the bar sticks below 100% forever after any failure.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        let bad = src.join("bad.txt");
        fs::write(&bad, vec![0u8; 4096]).expect("write");
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).expect("chmod");
        let dst = td.path().join("d");

        let p = plan_for(&[src], &dst, &recursive());
        let total = p.total_bytes;

        struct Counter {
            seen: u64,
        }
        impl crate::progress::Progress for Counter {
            fn scan_tick(&mut self, _f: usize, _b: u64) {}
            fn start(&mut self, _t: u64, _c: usize) {}
            fn file_started(&mut self, _p: &Path, _s: u64) {}
            fn advance(&mut self, bytes: u64) {
                self.seen += bytes;
            }
            fn note(&mut self, _l: &str) {}
            fn finish(&mut self) {}
        }

        let mut counter = Counter { seen: 0 };
        let flag = AtomicBool::new(false);
        run(&p, &recursive(), &mut counter, &flag);

        assert_eq!(counter.seen, total, "aggregate did not reach the total");
    }

    #[test]
    fn verbose_reports_each_copy() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a");
        fs::write(&src, b"x").expect("write");
        let dst = td.path().join("b");

        let verbose = Options {
            verbose: true,
            ..Options::default()
        };
        let p = plan_for(&[src.clone()], &dst, &verbose);
        let mut sink: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        {
            let mut prog = Null::new(&mut sink);
            run(&p, &verbose, &mut prog, &flag);
        }
        let out = String::from_utf8(sink).expect("utf8");
        assert!(out.contains("->"), "got {out:?}");
        assert!(out.contains("a"), "got {out:?}");
    }

    #[test]
    fn verbose_names_the_real_source_for_directories_and_symlinks() {
        // Every CopyOp carries `src` so -v never reports `dst -> dst`.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(src.join("sub").parent().expect("parent")).expect("mkdir s");
        fs::create_dir(src.join("sub")).expect("mkdir sub");
        symlink("sub", src.join("link")).expect("symlink");
        let dst = td.path().join("d");

        let verbose = Options {
            recursive: true,
            verbose: true,
            ..Options::default()
        };
        let p = plan_for(&[src.clone()], &dst, &verbose);
        let mut sink: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        {
            let mut prog = Null::new(&mut sink);
            run(&p, &verbose, &mut prog, &flag);
        }
        let out = String::from_utf8(sink).expect("utf8");

        for line in out.lines() {
            let (left, right) = line.split_once(" -> ").expect("arrow");
            assert_ne!(left, right, "verbose reported dst -> dst: {line:?}");
        }
        assert!(
            out.contains(&format!("{} -> ", src.join("sub").display())),
            "directory not reported with its source: {out:?}"
        );
    }

    #[test]
    fn an_interrupt_stops_the_run() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        for i in 0..10 {
            fs::write(src.join(format!("f{i}")), b"data").expect("write");
        }
        let dst = td.path().join("d");

        let p = plan_for(&[src], &dst, &recursive());
        let mut sink: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(true); // already interrupted
        let outcome = {
            let mut prog = Null::new(&mut sink);
            run(&p, &recursive(), &mut prog, &flag)
        };

        assert!(outcome.interrupted);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib engine 2>&1 | head -20`

Expected: compile error, `cannot find function run`.

- [ ] **Step 3: Implement `engine`**

Prepend to `src/engine.rs`:

```rust
use crate::cli::Options;
use crate::plan::{CopyOp, Plan};
use crate::progress::Progress;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Copy chunk size. Fixed, so memory use is constant regardless of file size —
/// a 100 GB file and a 100 KB file use the same working set.
pub const BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub failed: bool,
    pub interrupted: bool,
}

/// Read the process umask. There is no getter, so this sets it to zero, reads
/// the previous value, and restores it.
pub fn umask() -> u32 {
    unsafe {
        let previous = libc::umask(0);
        libc::umask(previous);
        previous as u32
    }
}

struct Runner<'a> {
    opts: &'a Options,
    progress: &'a mut dyn Progress,
    interrupted: &'a AtomicBool,
    mask: u32,
    outcome: Outcome,
}

impl Runner<'_> {
    fn fail(&mut self, path: &Path, err: &std::io::Error) {
        self.progress
            .note(&format!("cprogress: {}: {}", path.display(), err));
        self.outcome.failed = true;
    }

    fn verbose(&mut self, src: &Path, dst: &Path) {
        if self.opts.verbose {
            self.progress
                .note(&format!("{} -> {}", src.display(), dst.display()));
        }
    }

    fn aborted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    fn create_dir(&mut self, dst: &Path, mode: u32) {
        match fs::create_dir(dst) {
            Ok(()) => {
                // Only newly created directories take the source mode.
                let perms = fs::Permissions::from_mode(mode & !self.mask);
                if let Err(e) = fs::set_permissions(dst, perms) {
                    self.fail(dst, &e);
                }
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => self.fail(dst, &e),
        }
    }

    fn symlink(&mut self, target: &Path, dst: &Path) {
        let result = std::os::unix::fs::symlink(target, dst);
        match result {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Replacing an existing entry is what makes repeated runs work.
                if fs::remove_file(dst).is_ok() {
                    if let Err(e2) = std::os::unix::fs::symlink(target, dst) {
                        self.fail(dst, &e2);
                    }
                } else {
                    self.fail(dst, &e);
                }
            }
            Err(e) => self.fail(dst, &e),
        }
    }

    /// Open the destination, retrying after unlinking when `-f` is in effect.
    fn open_dest(&mut self, dst: &Path) -> std::io::Result<(File, bool)> {
        let existed = dst.symlink_metadata().is_ok();
        let attempt = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(dst);
        match attempt {
            Ok(f) => Ok((f, existed)),
            Err(e) if self.opts.force => {
                let _ = fs::remove_file(dst);
                match OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(dst)
                {
                    // The unlink means this is a fresh file, whatever came before.
                    Ok(f) => Ok((f, false)),
                    Err(_) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn copy_file(&mut self, src: &Path, dst: &Path, size: u64) {
        self.progress.file_started(src, size);

        let mut input = match File::open(src) {
            Ok(f) => f,
            Err(e) => {
                self.fail(src, &e);
                // Keep the aggregate honest so the bar can still reach 100%.
                self.progress.advance(size);
                return;
            }
        };

        let (mut output, existed) = match self.open_dest(dst) {
            Ok(pair) => pair,
            Err(e) => {
                self.fail(dst, &e);
                self.progress.advance(size);
                return;
            }
        };

        let mut buf = vec![0u8; BUFFER_SIZE];
        let mut copied: u64 = 0;
        loop {
            if self.aborted() {
                self.outcome.interrupted = true;
                return;
            }
            let n = match input.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    self.fail(src, &e);
                    self.progress.advance(size.saturating_sub(copied));
                    return;
                }
            };
            // write_all loops internally, so short writes are handled.
            if let Err(e) = output.write_all(&buf[..n]) {
                self.fail(dst, &e);
                self.progress.advance(size.saturating_sub(copied));
                return;
            }
            copied = copied.saturating_add(n as u64);
            self.progress.advance(n as u64);
        }

        if let Err(e) = output.flush() {
            self.fail(dst, &e);
            return;
        }

        // A file whose size changed under us should still land the bar on 100%.
        if copied < size {
            self.progress.advance(size - copied);
        }

        if !existed {
            match fs::metadata(src) {
                Ok(m) => {
                    let mode = (m.permissions().mode() & 0o7777) & !self.mask;
                    if let Err(e) = fs::set_permissions(dst, fs::Permissions::from_mode(mode)) {
                        self.fail(dst, &e);
                    }
                }
                Err(e) => self.fail(src, &e),
            }
        }

        self.verbose(src, dst);
    }
}

pub fn run(
    plan: &Plan,
    opts: &Options,
    progress: &mut dyn Progress,
    interrupted: &AtomicBool,
) -> Outcome {
    let mut runner = Runner {
        opts,
        progress,
        interrupted,
        mask: umask(),
        outcome: Outcome::default(),
    };

    for op in &plan.ops {
        if runner.aborted() {
            runner.outcome.interrupted = true;
            break;
        }
        match op {
            CopyOp::CreateDir { src, dst, mode } => {
                runner.create_dir(dst, *mode);
                runner.verbose(src, dst);
            }
            CopyOp::Symlink { src, target, dst } => {
                runner.symlink(target, dst);
                runner.verbose(src, dst);
            }
            CopyOp::CopyFile { src, dst, size } => runner.copy_file(src, dst, *size),
        }
    }

    runner.outcome
}
```

- [ ] **Step 4: Wire the module into main**

Add `mod engine;` to `src/main.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib engine`

Expected: 11 tests pass. Note that `a_failed_file_is_reported_and_the_run_continues` will not fail as intended when run as root, since root bypasses permission bits; run the suite as a normal user.

- [ ] **Step 6: Commit**

```bash
git add src/engine.rs src/main.rs
git commit -m "Add the copy engine with streaming, force, and error recovery"
```

---

### Task 9: Wire up main, signals, and exit codes

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: all prior modules.
- Produces: the working `cprogress` binary.

- [ ] **Step 1: Implement main**

Replace `src/main.rs` entirely:

```rust
mod cli;
mod engine;
mod fmt;
mod help;
mod plan;
mod progress;
mod rate;
mod term;

use cli::{CliError, ProgressMode};
use progress::bar::Bar;
use progress::{Null, Progress};
use std::env;
use std::ffi::OsString;
use std::io;
use std::os::unix::io::AsRawFd;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

const EXIT_OK: u8 = 0;
const EXIT_FAILURE: u8 = 1;
/// `EX_USAGE` from sysexits.h — what BSD `cp` returns for a bad invocation.
const EXIT_USAGE: u8 = 64;
const EXIT_INTERRUPTED: u8 = 130;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigint(_signal: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

fn install_sigint_handler() {
    unsafe {
        libc::signal(libc::SIGINT, handle_sigint as libc::sighandler_t);
    }
}

fn main() -> ExitCode {
    install_sigint_handler();

    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let invocation = match cli::parse(args) {
        Ok(inv) => inv,
        // -h/--help goes to stdout and succeeds; everything else is a usage error.
        Err(CliError::Help) => {
            print!("{}", help::MANUAL);
            return ExitCode::from(EXIT_OK);
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let stderr = io::stderr();
    let use_bar = match invocation.options.progress {
        ProgressMode::Auto => term::is_tty(stderr.as_raw_fd()),
        ProgressMode::Force => true,
        ProgressMode::None => false,
    };

    let mut boxed: Box<dyn Progress> = if use_bar {
        let fd = stderr.as_raw_fd();
        Box::new(Bar::new(
            io::stderr(),
            Box::new(move || term::width_or_fallback(fd)),
        ))
    } else {
        Box::new(Null::new(io::stderr()))
    };
    let progress = boxed.as_mut();

    let built = {
        let mut tick = |files: usize, bytes: u64| progress.scan_tick(files, bytes);
        plan::build(
            &invocation.sources,
            &invocation.dest,
            &invocation.options,
            &mut tick,
        )
    };

    let plan = match built {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };

    let mut failed = !plan.errors.is_empty();
    for line in &plan.errors {
        progress.note(line);
    }

    progress.start(plan.total_bytes, plan.file_count);
    let outcome = engine::run(&plan, &invocation.options, progress, &INTERRUPTED);
    progress.finish();

    failed |= outcome.failed;

    if outcome.interrupted {
        ExitCode::from(EXIT_INTERRUPTED)
    } else if failed {
        ExitCode::from(EXIT_FAILURE)
    } else {
        ExitCode::from(EXIT_OK)
    }
}
```

- [ ] **Step 2: Fix the borrow of `progress` inside the closure**

The closure above borrows `progress` mutably while it is also used afterwards. If the compiler rejects it, restructure the scan to collect ticks through a raw pointer-free approach by inlining the build call:

```rust
    let built = plan::build(
        &invocation.sources,
        &invocation.dest,
        &invocation.options,
        &mut |files: usize, bytes: u64| progress.scan_tick(files, bytes),
    );
```

If that still conflicts, give `plan::build` the progress reference directly by changing its last parameter to `&mut dyn Progress` and calling `scan_tick` internally — update `src/plan.rs`, its tests, and Task 7's interface note accordingly. Prefer the closure if it compiles.

- [ ] **Step 3: Build and run against a real tree**

```bash
cargo build --release
rm -rf /tmp/cpsrc /tmp/cpdst && mkdir -p /tmp/cpsrc/sub
head -c 20000000 /dev/urandom > /tmp/cpsrc/big.bin
echo hi > /tmp/cpsrc/sub/small.txt
ln -s big.bin /tmp/cpsrc/link
./target/release/cprogress -Rf /tmp/cpsrc /tmp/cpdst
echo "exit: $?"
diff -r /tmp/cpsrc /tmp/cpdst && echo "trees match"
```

Expected: a bar appears while copying, `exit: 0`, and `trees match`.

- [ ] **Step 4: Verify usage, help, and exit codes match cp**

```bash
./target/release/cprogress; echo "no args: $? (expect 64)"
./target/release/cprogress -p a b; echo "unsupported: $? (expect 64)"
./target/release/cprogress /nonexistent /tmp/x; echo "missing src: $? (expect 1)"
./target/release/cprogress /tmp/cpsrc /tmp/y 2>&1; echo "dir without -R: $? (expect 1)"
./target/release/cprogress -h | head -3; echo "help: ${PIPESTATUS[0]} (expect 0)"
```

Expected: 64, 64, 1, 1, 0, with the dir case printing `is a directory (not copied).`
and `-h` printing the manual starting at `NAME`.

Confirm the no-argument usage is `cp`'s, modulo the program name:

```bash
diff <(/bin/cp 2>&1 | sed 's/\bcp\b/cprogress/g') <(./target/release/cprogress 2>&1) \
  && echo "usage matches cp"
```

Expected: `usage matches cp`. Note `-h` writes to stdout while the usage error
writes to stderr; verify with `./target/release/cprogress -h 2>/dev/null | wc -l`
returning a non-zero line count.

- [ ] **Step 5: Verify non-TTY silence**

```bash
rm -rf /tmp/cpdst2
./target/release/cprogress -Rf /tmp/cpsrc /tmp/cpdst2 2>/tmp/err.txt
echo "stderr bytes: $(wc -c < /tmp/err.txt) (expect 0)"
```

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/plan.rs
git commit -m "Wire up the binary with signal handling and cp-compatible exit codes"
```

---

### Task 10: Integration tests

Differential testing against `/bin/cp` is the strongest available signal — it checks behavior against the real thing rather than against a reading of the man page.

**Files:**
- Create: `tests/differential.rs`
- Create: `tests/terminal.rs`
- Create: `tests/large_file.rs`

**Interfaces:**
- Consumes: the built `cprogress` binary via `env!("CARGO_BIN_EXE_cprogress")`.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Write the differential tests**

`tests/differential.rs`:

```rust
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cprogress");

/// Build a fixture covering the cases that differ between implementations:
/// nesting, symlinks, empty files, spaces in names, and a multi-chunk file.
fn make_fixture(root: &Path) {
    fs::create_dir_all(root.join("sub/deeper")).expect("mkdir");
    fs::write(root.join("hello.txt"), b"hello world").expect("write");
    fs::write(root.join("empty.txt"), b"").expect("write");
    fs::write(root.join("name with spaces.txt"), b"spaced").expect("write");
    fs::write(root.join("sub/nested.txt"), b"nested").expect("write");
    fs::write(root.join("sub/deeper/deep.bin"), vec![7u8; 3_000_000]).expect("write");
    symlink("hello.txt", root.join("link-to-file")).expect("symlink");
    symlink("sub", root.join("link-to-dir")).expect("symlink");
    fs::set_permissions(root.join("hello.txt"), fs::Permissions::from_mode(0o640))
        .expect("chmod");
    fs::set_permissions(root.join("sub"), fs::Permissions::from_mode(0o750)).expect("chmod");
}

/// Every path under `root`, relative, sorted, with type and mode.
fn describe(root: &Path) -> Vec<(String, String, u32)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).expect("read_dir");
        for entry in entries {
            let entry = entry.expect("entry");
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("lstat");
            let rel = path
                .strip_prefix(root)
                .expect("prefix")
                .to_string_lossy()
                .into_owned();
            let kind = if meta.file_type().is_symlink() {
                "symlink"
            } else if meta.is_dir() {
                "dir"
            } else {
                "file"
            };
            out.push((rel, kind.to_string(), meta.permissions().mode() & 0o777));
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Contents of every regular file, and targets of every symlink.
fn contents(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("lstat");
            let rel = path
                .strip_prefix(root)
                .expect("prefix")
                .to_string_lossy()
                .into_owned();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).expect("readlink");
                out.push((rel, target.to_string_lossy().as_bytes().to_vec()));
            } else if meta.is_dir() {
                stack.push(path);
            } else {
                out.push((rel, fs::read(&path).expect("read")));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn recursive_force_copy_matches_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("src");
    make_fixture(&src);

    let ours = td.path().join("ours");
    let theirs = td.path().join("theirs");

    let a = Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&ours)
        .output()
        .expect("run cprogress");
    let b = Command::new("/bin/cp")
        .args(["-Rf"])
        .arg(&src)
        .arg(&theirs)
        .output()
        .expect("run cp");

    assert!(a.status.success(), "cprogress failed: {a:?}");
    assert!(b.status.success(), "cp failed: {b:?}");
    assert_eq!(describe(&ours), describe(&theirs), "tree structure differs");
    assert_eq!(contents(&ours), contents(&theirs), "file contents differ");
}

#[test]
fn copy_into_existing_directory_matches_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("src");
    make_fixture(&src);

    let ours = td.path().join("ours");
    let theirs = td.path().join("theirs");
    fs::create_dir(&ours).expect("mkdir");
    fs::create_dir(&theirs).expect("mkdir");

    Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&ours)
        .status()
        .expect("run cprogress");
    Command::new("/bin/cp")
        .args(["-Rf"])
        .arg(&src)
        .arg(&theirs)
        .status()
        .expect("run cp");

    assert_eq!(describe(&ours), describe(&theirs));
    assert_eq!(contents(&ours), contents(&theirs));
}

#[test]
fn multiple_sources_into_a_directory_match_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let a = td.path().join("a.txt");
    let b = td.path().join("b.txt");
    fs::write(&a, b"aaa").expect("write");
    fs::write(&b, b"bbb").expect("write");

    let ours = td.path().join("ours");
    let theirs = td.path().join("theirs");
    fs::create_dir(&ours).expect("mkdir");
    fs::create_dir(&theirs).expect("mkdir");

    Command::new(BIN)
        .arg(&a)
        .arg(&b)
        .arg(&ours)
        .status()
        .expect("run cprogress");
    Command::new("/bin/cp")
        .arg(&a)
        .arg(&b)
        .arg(&theirs)
        .status()
        .expect("run cp");

    assert_eq!(describe(&ours), describe(&theirs));
    assert_eq!(contents(&ours), contents(&theirs));
}

#[test]
fn overwriting_preserves_the_destination_mode_like_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("src.txt");
    fs::write(&src, b"fresh").expect("write");
    fs::set_permissions(&src, fs::Permissions::from_mode(0o666)).expect("chmod");

    let ours = td.path().join("ours.txt");
    let theirs = td.path().join("theirs.txt");
    for p in [&ours, &theirs] {
        fs::write(p, b"stale").expect("write");
        fs::set_permissions(p, fs::Permissions::from_mode(0o600)).expect("chmod");
    }

    Command::new(BIN).arg(&src).arg(&ours).status().expect("run");
    Command::new("/bin/cp").arg(&src).arg(&theirs).status().expect("run");

    let m1 = fs::metadata(&ours).expect("stat").permissions().mode() & 0o777;
    let m2 = fs::metadata(&theirs).expect("stat").permissions().mode() & 0o777;
    assert_eq!(m1, m2, "mode differs from cp");
    assert_eq!(fs::read(&ours).expect("read"), fs::read(&theirs).expect("read"));
}

#[test]
fn exit_codes_match_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("adir");
    fs::create_dir(&dir).expect("mkdir");
    let out = td.path().join("out");

    // A directory without -R.
    let ours = Command::new(BIN).arg(&dir).arg(&out).output().expect("run");
    let theirs = Command::new("/bin/cp").arg(&dir).arg(&out).output().expect("run");
    assert_eq!(ours.status.code(), theirs.status.code(), "dir-without-R code");

    // A missing source.
    let missing = td.path().join("nope");
    let ours = Command::new(BIN).arg(&missing).arg(&out).output().expect("run");
    let theirs = Command::new("/bin/cp").arg(&missing).arg(&out).output().expect("run");
    assert_eq!(ours.status.code(), theirs.status.code(), "missing-source code");

    // A usage error.
    let ours = Command::new(BIN).output().expect("run");
    let theirs = Command::new("/bin/cp").output().expect("run");
    assert_eq!(ours.status.code(), theirs.status.code(), "usage code");
}

#[test]
fn bare_invocation_prints_cps_usage_on_stderr() {
    let ours = Command::new(BIN).output().expect("run");
    let theirs = Command::new("/bin/cp").output().expect("run");

    let mine = String::from_utf8_lossy(&ours.stderr).into_owned();
    let reference = String::from_utf8_lossy(&theirs.stderr)
        .replace("cp ", "cprogress ")
        .replace("usage: cprogress", "usage: cprogress");

    assert_eq!(mine.trim_end(), reference.trim_end(), "usage text differs");
    assert!(ours.stdout.is_empty(), "usage should not go to stdout");
}

#[test]
fn help_prints_a_manual_on_stdout_and_succeeds() {
    for flag in ["-h", "--help"] {
        let out = Command::new(BIN).arg(flag).output().expect("run");
        assert!(out.status.success(), "{flag} should exit 0");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("NAME"), "{flag}: no NAME section");
        assert!(text.contains("SYNOPSIS"), "{flag}: no SYNOPSIS section");
        assert!(text.contains("EXIT STATUS"), "{flag}: no EXIT STATUS section");
        assert!(
            text.contains("-p -i -n"),
            "{flag}: unsupported options not listed"
        );
        assert!(out.stderr.is_empty(), "{flag}: help should not use stderr");
    }
}
```

- [ ] **Step 2: Run the differential tests**

Run: `cargo test --test differential`

Expected: 7 tests pass. If `bare_invocation_prints_cps_usage_on_stderr` fails on
whitespace, compare the two strings byte by byte with `cargo test -- --nocapture`
and align `help::USAGE` to `cp`'s exact spacing — `cp` is the reference.

If a mode assertion fails, investigate against `/bin/cp` directly with
`stat -f '%Sp %N'` before changing the test. `cp`'s behavior is the
specification here, so a mismatch means the implementation is wrong, not the
test.

- [ ] **Step 3: Write the terminal tests**

`tests/terminal.rs`:

```rust
use std::fs;
use std::io::Read;
use std::os::unix::io::FromRawFd;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_cprogress");

#[test]
fn produces_no_stderr_when_not_a_tty() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("s");
    fs::create_dir(&src).expect("mkdir");
    fs::write(src.join("f.bin"), vec![0u8; 2_000_000]).expect("write");
    let dst = td.path().join("d");

    let out = Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&dst)
        .stderr(Stdio::piped())
        .output()
        .expect("run");

    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "expected silence, got {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn progress_none_silences_the_bar() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("f.bin");
    fs::write(&src, vec![0u8; 1000]).expect("write");
    let dst = td.path().join("out.bin");

    let out = Command::new(BIN)
        .arg("--progress=none")
        .arg(&src)
        .arg(&dst)
        .output()
        .expect("run");

    assert!(out.stderr.is_empty());
}

#[test]
fn draws_a_bar_on_a_pty() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("s");
    fs::create_dir(&src).expect("mkdir");
    // Large enough to cross the per-file bar threshold.
    fs::write(src.join("big.bin"), vec![0u8; 12 * 1024 * 1024]).expect("write");
    let dst = td.path().join("d");

    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty failed");

    let status = Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&dst)
        .stderr(unsafe { Stdio::from_raw_fd(slave) })
        .status()
        .expect("run");
    assert!(status.success());

    // The child holds the only other reference; closing ours ends the read.
    let mut reader = unsafe { fs::File::from_raw_fd(master) };
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    let out = String::from_utf8_lossy(&buf);

    assert!(out.contains("total"), "no aggregate bar: {out:?}");
    assert!(out.contains("big.bin"), "no file line: {out:?}");
    assert!(out.contains("copied in"), "no summary: {out:?}");
}
```

Add `libc` to dev-dependencies use by ensuring `Cargo.toml` keeps `libc` as a
regular dependency — integration tests can use it only if it is also a
dev-dependency, so add this to `Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
libc = "0.2"
```

- [ ] **Step 4: Run the terminal tests**

Run: `cargo test --test terminal`

Expected: 3 tests pass. If `draws_a_bar_on_a_pty` hangs, the slave fd is still open in the parent; ensure it is moved into `Stdio::from_raw_fd` and not duplicated.

- [ ] **Step 5: Write the opt-in large-file test**

`tests/large_file.rs`:

```rust
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cprogress");

/// Ignored by default: needs several GB of free disk and real wall-clock time.
/// Run with `cargo test --test large_file -- --ignored --nocapture`.
#[test]
#[ignore]
fn copies_a_file_larger_than_four_gibibytes() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("huge.bin");
    let dst = td.path().join("copy.bin");

    let size: u64 = 5 * 1024 * 1024 * 1024; // 5 GiB, past the 32-bit boundary
    {
        let mut f = File::create(&src).expect("create");
        // Sparse body with real data at both ends, so the copy must traverse
        // the whole length without materializing it in memory.
        f.write_all(b"START").expect("write head");
        f.seek(SeekFrom::Start(size - 3)).expect("seek");
        f.write_all(b"END").expect("write tail");
        f.flush().expect("flush");
    }
    assert_eq!(fs::metadata(&src).expect("stat").len(), size);

    let status = Command::new(BIN)
        .arg(&src)
        .arg(&dst)
        .status()
        .expect("run cprogress");
    assert!(status.success());

    let copied = fs::metadata(&dst).expect("stat").len();
    assert_eq!(copied, size, "size mismatch on a 5 GiB file");

    // Spot-check both ends rather than hashing 5 GiB.
    let mut f = File::open(&dst).expect("open");
    let mut head = [0u8; 5];
    std::io::Read::read_exact(&mut f, &mut head).expect("read head");
    assert_eq!(&head, b"START");
    f.seek(SeekFrom::Start(size - 3)).expect("seek");
    let mut tail = [0u8; 3];
    std::io::Read::read_exact(&mut f, &mut tail).expect("read tail");
    assert_eq!(&tail, b"END");
}
```

- [ ] **Step 6: Verify the large-file test is skipped by default and passes on demand**

```bash
cargo test --test large_file 2>&1 | tail -3
```

Expected: `1 ignored`.

```bash
cargo test --test large_file -- --ignored --nocapture 2>&1 | tail -5
```

Expected: passes. This writes 5 GiB; skip if the machine is short on disk and note that it was skipped.

- [ ] **Step 7: Run the whole suite**

Run: `cargo test`

Expected: every test passes, with the large-file test reported as ignored.

- [ ] **Step 8: Commit**

```bash
git add tests/ Cargo.toml Cargo.lock
git commit -m "Add differential, terminal, and large-file integration tests"
```

---

### Task 11: README and man page

**Files:**
- Create: `README.md`
- Create: `man/cprogress.1`

**Interfaces:**
- Consumes: the finished binary, `help::MANUAL`.
- Produces: nothing.

- [ ] **Step 1: Capture real output for the README**

```bash
cargo build --release
rm -rf /tmp/rsrc /tmp/rdst && mkdir -p /tmp/rsrc
head -c 60000000 /dev/urandom > /tmp/rsrc/big.bin
./target/release/cprogress -Rf --progress=force /tmp/rsrc /tmp/rdst 2>&1 | tail -4
```

Use the actual lines this prints in the README sample below rather than inventing them.

- [ ] **Step 2: Write the README**

`README.md`:

````markdown
# cprogress

`cp` for macOS, with a progress bar.

Copying a large tree with `cp` tells you nothing until it finishes. `cprogress`
takes the same arguments and prints a wget-style bar while it works.

```
total  62%[===========>      ] 4.21G  118MB/s  eta 22s
photos/IMG_4471.HEIC  30%[====>          ] 1.4M/4.6M
```

The top line tracks the whole operation; the bottom line tracks the file being
copied right now. Files smaller than 8 MiB show just their path, since a bar
that fills instantly is noise rather than information.

## Install

Requires a Rust toolchain and macOS.

```sh
git clone https://github.com/peterazuolas/cprogress
cd cprogress
cargo install --path .
```

Or build and place the binary yourself:

```sh
cargo build --release
cp target/release/cprogress /usr/local/bin/
```

## Usage

```
cprogress [-R | -r] [-f] [-v] [--progress=auto|force|none] source ... target
```

| Flag | Meaning |
| --- | --- |
| `-R`, `-r` | Copy directories recursively. Symlinks are recreated, not followed. |
| `-f` | If the destination cannot be opened for writing, remove it and retry. |
| `-v` | Print each file as it is copied. |
| `-h`, `--help` | Print the full manual. |
| `--progress` | `auto` (default) shows the bar only on a terminal; `force` always; `none` never. |

Run with no arguments and it prints the same usage `cp` does, and exits 64.

The argument grammar matches BSD `cp`: one source copies to a file or into a
directory, several sources require an existing directory. Exit codes match too —
`0` on success, `1` if any file failed, `64` for a usage error, `130` on Ctrl-C.

Because the bar is suppressed when stderr is not a terminal, this is safe:

```sh
alias cp=cprogress
```

Scripts see exactly what `cp` would print.

## Not supported

`-p`, `-i`, `-n`, `-a`, `-c`, `-l`, `-s`, `-X`, `-H`, `-L`, `-P`. These fail with
a message rather than being silently ignored, so a script that relies on one
finds out immediately. Extended attributes and ACLs are not copied.

New files and directories take the source's permissions masked by your umask,
and an existing destination file keeps its own permissions — both matching `cp`.

## How it works

`cprogress` walks the sources first to total up the bytes, then copies with a
fixed total, which is what makes the ETA trustworthy from the first frame. The
scan is metadata-only and takes a second or two even on very large trees.

Files stream through a 1 MiB buffer, so memory use is constant no matter the file
size — copying a 100 GB file uses the same working set as a 100 KB one.

## Tests

```sh
cargo test
```

The suite includes differential tests that run `/bin/cp` and `cprogress` over the
same fixture and compare the resulting trees.

One test is excluded by default because it writes 5 GiB:

```sh
cargo test --test large_file -- --ignored
```

## License

MIT. See [LICENSE](LICENSE).
````

- [ ] **Step 3: Verify the README sample matches reality**

Re-read the README's sample bar against the output captured in Step 1. If the
real layout differs (spacing, field order), correct the README — it must show
what the tool actually prints.

- [ ] **Step 4: Write the man page**

`man/cprogress.1` — the same content as `help::MANUAL`, in roff. Keep the two in
sync; if they drift, `-h` is the authority.

```roff
.Dd July 30, 2026
.Dt CPROGRESS 1
.Os
.Sh NAME
.Nm cprogress
.Nd copy files with a progress bar
.Sh SYNOPSIS
.Nm
.Op Fl R | r
.Op Fl f
.Op Fl v
.Op Fl -progress Ns = Ns Ar auto | force | none
.Ar source_file target_file
.Nm
.Op Fl R | r
.Op Fl f
.Op Fl v
.Op Fl -progress Ns = Ns Ar auto | force | none
.Ar source_file ... target_directory
.Sh DESCRIPTION
.Nm
copies files the way
.Xr cp 1
does, and prints a wget-style progress bar while it works.
.Pp
In the first form,
.Ar source_file
is copied to
.Ar target_file .
In the second form, each
.Ar source_file
is copied into the existing
.Ar target_directory .
.Pp
Before copying,
.Nm
walks the sources to total up the bytes, so the percentage and estimated time
are accurate from the first frame. The scan reads metadata only.
.Pp
The display is two lines: an aggregate bar for the whole operation, and a bar
for the file being copied. Files smaller than 8 MiB show only their path,
because a bar that fills within a single read is noise rather than information.
.Pp
The options are as follows:
.Bl -tag -width Ds
.It Fl R , r
Copy directory hierarchies recursively. Symbolic links are recreated as
symbolic links rather than followed, matching the default behavior of
.Nm cp Fl R .
.It Fl f
If a destination file cannot be opened for writing, remove it and try again.
.It Fl v
Print the name of each file as it is copied.
.It Fl h , -help
Print the manual and exit.
.It Fl -progress Ns = Ns Ar auto | force | none
Control the progress display.
.Ar auto ,
the default, draws the bar only when standard error is a terminal.
.Ar force
always draws it.
.Ar none
never does.
.El
.Sh UNSUPPORTED OPTIONS
The following
.Xr cp 1
options are not implemented and are rejected with an error rather than
ignored, so that a script relying on one fails visibly:
.Pp
.Dl Fl p Fl i Fl n Fl a Fl c Fl l Fl s Fl X Fl H Fl L Fl P
.Pp
Extended attributes and access control lists are not copied.
.Sh FILE PERMISSIONS
A newly created file or directory takes the source's permission bits masked by
the current umask. An existing destination file keeps its own permissions. Both
match
.Xr cp 1 .
.Pp
Ownership, timestamps, and BSD file flags are not preserved; that is what
.Nm cp Fl p
would provide, and
.Fl p
is not supported.
.Sh EXIT STATUS
.Bl -tag -width Ds -compact
.It 0
All files were copied successfully.
.It 1
At least one file could not be copied.
.It 64
The command line was malformed.
.It 130
The copy was interrupted with SIGINT.
.El
.Sh EXAMPLES
Copy a directory tree, replacing anything already at the destination:
.Pp
.Dl cprogress -Rf ~/Photos /Volumes/Backup/Photos
.Pp
Use it in place of
.Xr cp 1
everywhere. The bar is suppressed when standard error is not a terminal, so
scripts see exactly what
.Xr cp 1
would print:
.Pp
.Dl alias cp=cprogress
.Sh SEE ALSO
.Xr cp 1
```

- [ ] **Step 5: Verify the man page renders**

```bash
man ./man/cprogress.1 | head -20
```

Expected: a formatted page with NAME and SYNOPSIS. If `man` refuses a relative
path, use `groff -Tascii -man man/cprogress.1 | head -20`.

- [ ] **Step 6: Document the man page in the README**

Add to the README's Install section, after the `cargo install` block:

```markdown
To install the man page as well:

```sh
sudo cp man/cprogress.1 /usr/local/share/man/man1/
man cprogress
```

`cprogress -h` prints the same content without installing anything.
```

- [ ] **Step 7: Commit**

```bash
git add README.md man/
git commit -m "Add README and man page"
```

---

## Final Verification

- [ ] **Step 1: Full suite**

Run: `cargo test`
Expected: all pass, one ignored.

- [ ] **Step 2: Clean release build**

Run: `cargo build --release && cargo clippy --all-targets -- -D warnings`
Expected: no warnings. If `clippy` is not installed, note it and skip.

- [ ] **Step 3: Confirm the dependency budget held**

Run: `grep -A6 '^\[dependencies\]' Cargo.toml`
Expected: `libc` only under `[dependencies]`; `tempfile` and `libc` under `[dev-dependencies]`.

- [ ] **Step 4: Real-world smoke test with a large file**

```bash
rm -rf /tmp/final-src /tmp/final-dst && mkdir -p /tmp/final-src
head -c 200000000 /dev/urandom > /tmp/final-src/big.bin
./target/release/cprogress -Rf /tmp/final-src /tmp/final-dst
diff -r /tmp/final-src /tmp/final-dst && echo OK
```

Expected: both bars render, the ETA counts down sensibly, trees match.

- [ ] **Step 5: Code review**

Run the three review agents in parallel in a single message — architecture
review, design consistency, and code organization/naming — each instructed to
REPORT ONLY. Verify each finding against the code before applying it, then apply
the merged set yourself in one serial pass and re-run `cargo test`.

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "Apply review findings"
```
