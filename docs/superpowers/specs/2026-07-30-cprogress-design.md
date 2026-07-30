# cprogress — design

**Date:** 2026-07-30
**Status:** approved

## Purpose

A drop-in replacement for the common uses of macOS `cp` that prints a wget-style
progress bar while copying. The primary workload is `cp -Rf source destination`
on large directory trees, where `cp`'s silence gives no indication of whether the
copy will finish in ten seconds or forty minutes.

## Scope

Target platform is macOS (tested on 26.4, arm64). Written in Rust, distributed as
a single binary.

### Supported

```
cprogress [-R|-r] [-f] [-v] [--progress=auto|force|none] src... dest
```

- Argument grammar identical to BSD `cp`: one source copies to a file or into a
  directory; multiple sources require an existing directory destination.
- `-R`/`-r` — recursive. Symlinks are recreated as symlinks rather than followed,
  matching BSD's default (equivalent to `cp -RP`).
- `-f` — if the destination exists and cannot be opened for writing, unlink it
  and retry.
- `-v` — print `src -> dest` per file.
- `--progress` — `auto` (default: bar only when stderr is a TTY), `force`, `none`.
- Flag clustering (`-Rf`), the `--` terminator, and BSD trailing-slash behavior
  (a trailing slash on a source directory is ignored, unlike GNU `cp`).

### Deliberately excluded from v1

`-p`, `-i`, `-n`, `-a`, `-c`, `-l`, `-s`, `-X`, `-H`, `-L`, `-P`.

These are **hard errors with a clear message**, never silent no-ops:

```
cprogress: -p is not supported
```

Rationale: `-i` and `-n` only add prompting, and `cp`'s default is already silent
overwrite, so their absence does not change `-Rf` behavior. `-p` carries real
implementation weight (uid/gid, BSD file flags, `utimensat`) and is not needed by
the target workload. Failing loudly means a `-p` that appears in a script surfaces
immediately rather than silently producing a tree with wrong ownership.

Also excluded: parallel copying. It breaks per-file error ordering and diverges
from `cp` semantics for a speedup that is not the bottleneck. The copy loop is
single-threaded.

Extended attributes and ACLs are not copied. This is documented in the man page
and `--help`.

Note that excluding `-p` does not mean modes are ignored. As `cp` does without
`-p`, newly created files and directories take the source's mode bits masked by
the process umask. What `-p` would add — ownership, timestamps, and BSD file
flags — is what is absent.

## Approach

**Pre-scan, then copy.** The tool walks all sources first, summing file sizes,
then runs the copy against a fixed total.

The alternative — walking and copying concurrently, with the total growing as
files are discovered — starts moving bytes sooner but makes the denominator a
moving target: the percentage stalls or slides backwards and the ETA is
meaningless until the walk completes. That is the exact failure mode a wget-style
bar exists to avoid.

The pre-scan costs a metadata-only walk (roughly 1–3 s for a 500k-file tree on
APFS), which is cheap relative to the byte copy that follows, and it buys an
honest ETA from the first frame.

## Architecture

Four modules. Data flows one direction: `cli → plan → engine → progress`. No
shared mutable state, no upward callbacks. The copy engine knows nothing about
terminals.

### `cli`

`argv` → `Options`, `Vec<PathBuf>` sources, and a dest `PathBuf`. Pure; performs
no I/O. Tests are string-in, struct-out.

Hand-rolled, roughly 120 lines, with no `clap` dependency. `cp`'s grammar is small
but has specifics — positional/option interleaving, `--`, cluster parsing — that
are more directly expressed than coaxed out of a general-purpose parser.

### `plan`

Walks the sources and produces:

```rust
struct Plan {
    ops: Vec<CopyOp>,
    total_bytes: u64,
    file_count: usize,
}

enum CopyOp {
    CreateDir { dst: PathBuf, mode: u32 },
    CopyFile  { src: PathBuf, dst: PathBuf, size: u64 },
    Symlink   { target: PathBuf, dst: PathBuf },
}
```

Destination semantics (copy-into-directory vs copy-to-new-name) are resolved here,
so the engine never re-derives paths. Validation that must precede any write also
lives here: multiple sources with a non-directory destination, and a destination
nested inside its own source (which would recurse infinitely).

I/O is limited to `stat` and `readdir`.

### `engine`

Executes a `Plan` serially, emitting events to a `&mut dyn Progress`. Copies files
with a 1 MiB read/write buffer. Never writes to stderr directly — all output goes
through the `Progress` trait.

**No file-size bound.** Files are streamed through a fixed 1 MiB buffer, so memory
use is constant regardless of file size — a 100 GB file and a 100 KB file use the
same working set. Nothing is memory-mapped or read whole. Every byte count in the
system (`CopyOp::size`, `Plan::total_bytes`, the progress counters, the rate
estimator's accumulators) is `u64`, which tops out around 18 exabytes; there is no
32-bit truncation anywhere in the path. Rust's `std::fs` uses 64-bit `off_t` on
macOS, so large files need no special handling at the syscall layer.

Short writes are handled by looping until the buffer is fully written, rather than
assuming one `write` consumes it — this matters on network volumes, which is
exactly where 100 GB copies tend to happen.

### `progress`

```rust
trait Progress {
    fn scan_tick(&mut self, files: usize, bytes: u64);
    fn start(&mut self, total_bytes: u64, file_count: usize);
    fn file_started(&mut self, path: &Path);
    fn advance(&mut self, bytes: u64);
    fn note(&mut self, line: &str);
    fn finish(&mut self);
}
```

Two implementations: `Bar` (TTY) and `Null` (non-TTY, or `--progress=none`).

`note()` is what lets `-v` lines and error messages print *above* a live bar: it
clears the bar's lines, writes the message, and redraws.

### Dependencies

`libc` only. Terminal width comes from a `TIOCGWINSZ` ioctl re-queried on each
repaint, which handles window resizes without a `SIGWINCH` handler.

## Progress display

Two phases on stderr, each a repainting block.

Scan phase:

```
Scanning... 41,203 files, 88.2G
```

Copy phase — two lines, because a filename plus a full-width bar does not fit in
80 columns without gutting one of them:

```
photos/IMG_4471.HEIC
 62%[===========>       ] 4.21G  118MB/s  eta 22s
```

The path is shown relative to the source root, middle-ellipsized
(`photos/…/IMG_4471.HEIC`) when it exceeds the terminal width.

Completion summary:

```
88.2G copied in 12m41s (118MB/s)
```

### Mechanics

- Repaint throttled to 10 Hz, checked in the copy loop after each chunk. No timer
  thread.
- Transfer rate is an exponentially weighted moving average with a ~3 s half-life.
  A raw instantaneous rate makes the ETA jitter badly across mixed file sizes; a
  cumulative average reacts too slowly when the copy moves from large files into a
  directory of small ones.
- Non-TTY stderr produces no output at all, so `alias cp=cprogress` is safe in
  scripts.
- When `--progress=force` is used and stderr is not a TTY, `TIOCGWINSZ` fails and
  the width falls back to 80 columns.
- When the plan totals zero bytes (empty files, or only directories), the bar
  renders 100% rather than dividing by zero.
- Sizes are formatted with a unit scale reaching `T` and `P`, so a multi-terabyte
  total does not render as a five-digit gigabyte count.

### Signals

`SIGINT` sets an abort flag that the copy loop checks between chunks. On abort the
bar prints a trailing newline so the shell prompt lands cleanly, and the process
exits 130. The partially written destination file is left in place, matching `cp`.

## Error handling

`cp`'s model. A failure on one file prints to stderr and the run continues with the
next operation:

```
cprogress: /path/to/file: Permission denied
```

Exit status is 1 if any operation failed, 0 otherwise, 130 on `SIGINT`.

The pre-scan changes two things relative to `cp`:

- **Better:** unreadable sources are detected during the scan and reported before
  any bytes move.
- **Worse:** the total can be wrong if the tree changes underneath a running copy.
  When actual bytes exceed the total, the bar clamps at 100% and the ETA renders as
  `--`. Only the estimate degrades; the copy itself is unaffected.

## Testing

### Unit

Pure functions, no I/O:

- `cli` parsing: argv → `Options`, including cluster flags, `--`, and rejection of
  unsupported flags.
- Size and duration formatting.
- Bar rendering at terminal widths 40, 80, and 200.
- EWMA rate estimator against synthetic timing sequences.
- Byte arithmetic above 2^32: percentage, ETA, and size formatting fed values in
  the 100 GB–10 TB range, asserting no truncation or overflow. This covers the
  large-file math without moving large-file bytes.

### Differential integration

The strongest available signal: test against the real `cp` rather than against a
reading of its man page.

Build a fixture tree covering nested directories, symlinks, empty files, names
with spaces, and one large file. Run `/bin/cp -Rf` and `cprogress -Rf` into
separate destinations. Assert the resulting trees are byte-identical and that
modes match.

### Terminal behavior

- Non-TTY: run with stderr piped, assert it is empty.
- TTY: one smoke test over a `libc::openpty` pair asserting that a bar and the
  summary line appear.

### Large files

One `#[ignore]`-marked test, run on demand rather than in the default suite,
copies a real file larger than 4 GiB and asserts the destination's size and
checksum match. Kept opt-in because it needs meaningful disk space and wall-clock
time; the routine coverage for large-file correctness is the byte-arithmetic unit
test above.

Dev-dependencies: `tempfile`.

## Installation

`cargo install --path .`, or copy the release binary onto `PATH`.
