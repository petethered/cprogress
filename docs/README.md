# Design record

cprogress was built design-first with an AI coding agent (Claude Code). A spec
was written and approved first. Then came a task-by-task implementation plan,
executed by agents with a review after each task. A final review against
`/bin/cp` followed. Both documents are kept here unedited, as they were when
they drove the build.

| Document | What it is |
| --- | --- |
| [Design spec](superpowers/specs/2026-07-30-cprogress-design.md) | Scope, flag set, architecture (`cli → plan → engine → progress`), progress-display rules, error model, test strategy. Approved before any code was written. |
| [Implementation plan](superpowers/plans/2026-07-30-cprogress.md) | Eleven tasks, each with the files it touches, the tests to write first, and the commit it ends in. The header addresses the agents that executed it. |

The `superpowers/` path comes from the planning workflow
([obra/superpowers](https://github.com/obra/superpowers)) that generated these
documents. Working conventions for agents in this repo are in
[CLAUDE.md](../CLAUDE.md).

## How the build departed from the spec

The spec was written from `cp`'s documentation. The build was then tested
against `/bin/cp` itself, which turned up differences. In each case below, the
code now follows `cp`, and the commit body records what was verified.
Implementation-level departures, such as trait signatures and field names, are
not listed. The code is the reference for those.

**Behaviour now matching `cp`, which the spec had wrong or left unspecified**

- **Symlink sources without `-R` are followed.** The spec only covered `-R`.
  BSD `cp` preserves links only under `-R`, and dereferences them otherwise.
  (`0ab4cd6`)
- **`-v` writes to stdout, not stderr.** This keeps `cprogress -v src dst > log`
  equivalent to `cp`. (`183c883`)
- **Diagnostics match `cp` byte for byte.** There is no `(os error N)` suffix,
  and an unknown flag prints `illegal option -- z`. (`6cfb520`, `8b183b6`)
- **Destinations nested inside the source are allowed.** The spec rejected them
  to avoid infinite recursion, but the pre-scan finishes before anything is
  written, so that risk never applied. The check that remains is the one `cp`
  makes: source and destination are the same file (same device and inode).
  (`18f1afc`)
- **A failed directory prunes its subtree.** The result is one error, as with
  `cp`, instead of one per descendant. (`63f290a`, `f07837b`)
- **Read-only source directories keep their contents.** A directory's mode is
  applied after its children are copied, deepest first, as `cp` does.
  (`f07837b`)
- **`-f` unlinks only on `EACCES`.** It doesn't unlink on every open error.
  (`5cf4614`)

**Scope settled after the spec**

- **`-N`, `-S` and `-x` were added to the rejected flags.** They are documented
  BSD `cp` options that the spec's list missed. (`6cfb520`, `195c282`)
- **Sockets, FIFOs and device nodes are skipped with an error, and the exit
  status is 1.** The spec didn't cover special files. This is a documented
  divergence from `cp`. (`195c282`, `9b07feb`)
- **Copying a single file uses a one-line display.** The two-line layout would
  repeat the same numbers. (`6ef2b7a`)
