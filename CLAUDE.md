# CLAUDE.md

Instructions for AI coding agents working in this repository. Humans should
start with [README.md](README.md); the design record is under [docs/](docs/).

## What this is

`cprogress` is BSD `cp` for macOS with a wget-style progress bar. Rust 2021,
macOS only, `libc` as the only runtime dependency.

The guiding rule: **behave exactly like `/bin/cp`**, in arguments, exit codes,
diagnostics (wording and stream) and resulting files. Any difference must be
deliberate and documented. When unsure what `cp` does, run `/bin/cp` and look;
don't go by its man page or by memory.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                                     # includes differential tests against /bin/cp
cargo test --test large_file -- --ignored      # writes 5 GiB; only on request
mandoc -Tlint man/cprogress.1                  # after editing the man page
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, and tests on stable and on the
declared `rust-version`, all on macOS. All three must pass before a commit.

## Layout

Data flows one way: `cli → plan → engine → progress`.

| Module | Responsibility |
| --- | --- |
| `cli` | argv → `Options`. Pure; no I/O. Hand-rolled `cp` grammar (clusters, `--`). |
| `plan` | Pre-scans sources (stat/readdir only) into `CopyOp`s with a fixed byte total; resolves destination paths and validates everything that must fail before a write. |
| `engine` | Executes the plan serially through a 1 MiB buffer. Never writes to stderr itself; all output goes through `Progress`. |
| `progress` | `Progress` trait, `Null` and `Bar`. `render.rs` is pure string functions so layout is unit-tested without a terminal. |
| `rate`, `fmt`, `term` | EWMA rate estimator, byte/duration formatting, TTY and width detection. |
| `msg` | The `cprogress:` prefix and errno wording (no `(os error N)` suffix). |
| `help` | `USAGE` (mirrors `cp`'s) and `MANUAL` (the `-h` text). |

## Rules

- **Three documentation surfaces move together.** Any change to flags,
  diagnostics, or special-file behaviour updates `README.md`, `man/cprogress.1`,
  and `MANUAL` in `src/help.rs` in the same commit. Tests in `help.rs` and
  `cli.rs` fail if the unsupported-flag lists drift apart. Keep them passing by
  fixing the docs, not by weakening the tests.
- **Unsupported flags fail loudly.** A flag we don't implement goes in
  `UNSUPPORTED_SHORT` and gets a clear error. It is never silently ignored.
- **Pin exact text.** Tests on diagnostics assert the full line `cp` prints, not
  a substring. A substring assertion once hid operands printed in the wrong order.
- **Every byte count is `u64`.** Memory use stays constant regardless of file size.
- **No new dependencies** without a reason that `libc` and `std` cannot cover.
- **Tests come first** for bug fixes: reproduce with a failing test, ideally a
  differential one against `/bin/cp`, then fix.

## Commits

Subject plus body. The subject is imperative and has no type prefix. The body
explains *why*, and what was verified against `/bin/cp`. Make small, focused
commits. No AI attribution and no `Co-Authored-By` trailers.

## Documented differences from `cp`

These are deliberate. Don't "fix" them without discussion:

- Sockets, FIFOs and device nodes are skipped with an error and exit 1. `cp`
  recreates some of them under `-R` and exits 0.
- `-p -i -n -a -c -l -s -X -H -L -P -N -S -x` are rejected.
- Extended attributes, ACLs, ownership, timestamps and BSD flags are not copied.

See "Not supported" in the README for the full statement.
