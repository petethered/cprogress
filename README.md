# cprogress

`cp` for macOS, with a progress bar.

Copying a large tree with `cp` tells you nothing until it finishes. `cprogress`
takes the same arguments and prints a wget-style bar while it works.

```
total  44%[==================>                          ] 1.24G  279MB/s  eta 5s
Photos/vacation.mov  44%[=================>                        ] 1.24G/2.79G
```

(Captured from a real run: `cprogress -Rf --progress=force Photos /tmp/dst` on a
2.79 GiB directory.)

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

To install the man page as well:

```sh
sudo cp man/cprogress.1 /usr/local/share/man/man1/
man cprogress
```

`cprogress -h` prints the same content without installing anything.

## Usage

```
cprogress [-R | -r] [-f] [-v] [--progress=auto|force|none] source ... target
```

| Flag | Meaning |
| --- | --- |
| `-R`, `-r` | Copy directories recursively. Symlinks are recreated, not followed. Without `-R` a symlink is followed and its target copied, as `cp` does. |
| `-f` | If the destination cannot be opened for writing, remove it and retry. |
| `-v` | Print each file as it is copied, on stdout. Errors go to stderr, as with `cp`. |
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

`-p`, `-i`, `-n`, `-a`, `-c`, `-l`, `-s`, `-X`, `-H`, `-L`, `-P`, `-N`, `-S`,
`-x`. These fail with a message rather than being silently ignored, so a script
that relies on one finds out immediately. Extended attributes and ACLs are not
copied.

Only regular files, directories, and symlinks are copied. A socket, FIFO, or
device node is reported as an unsupported file type and skipped — the rest of
the tree still copies, and the exit status is 1. `cp -R` recreates these, so a
tree containing one is the one case where the two tools differ in what lands.

New files and directories take the source's permissions masked by your umask,
and an existing destination file keeps its own permissions — both matching `cp`.

Ownership, timestamps, and BSD file flags are not preserved; that is what
`cp -p` would provide, and `-p` is not supported.

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
