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

             Without -R, a symbolic link is followed and whatever it points at
             is copied, which is also what cp does. A link to a directory is
             therefore reported as a directory, and a dangling link as a
             missing file.

     -f      If a destination file cannot be opened for writing, remove it and
             try again.

     -v      Print the name of each file as it is copied, as "source -> dest",
             on standard output. Errors go to standard error. cp splits them
             the same way, so redirecting either stream behaves identically.

     -h      Print this manual and exit.

     --help  Same as -h.

     --progress=auto|force|none
             Control the progress display. auto, the default, draws the bar
             only when standard error is a terminal. force always draws it.
             none never does.

UNSUPPORTED OPTIONS
     The following cp(1) options are not implemented and are rejected with an
     error rather than ignored, so that a script relying on one fails visibly:

           -p -i -n -a -c -l -s -X -H -L -P -N -S -x

     Extended attributes and access control lists are not copied.

UNSUPPORTED FILE TYPES
     Only regular files, directories, and symbolic links are copied. A socket,
     FIFO, or device node is reported as an unsupported file type and skipped;
     the rest of the tree is still copied, and the exit status is 1. cp -R
     recreates these, so a tree containing one is a case where cprogress and
     cp differ.

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

    /// A user can read three things: `-h`, the man page, and the README. The
    /// review found all three drifting apart — the README was missing the
    /// ownership sentence the other two carried, and none of them mentioned
    /// the special-file limitation. Each is asserted in all three here so the
    /// next omission fails a test rather than reaching a user.
    #[test]
    fn every_doc_surface_carries_the_same_limitations() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let man = std::fs::read_to_string(root.join("man/cprogress.1")).expect("man page");
        let readme = std::fs::read_to_string(root.join("README.md")).expect("README");
        let surfaces = [("MANUAL", MANUAL), ("man page", man.as_str()), ("README", readme.as_str())];

        for (name, text) in surfaces {
            // The options this pass added to UNSUPPORTED_SHORT. The man page
            // spells them "Fl N"; the other two use a bare "-N".
            for flag in ['N', 'S', 'x'] {
                assert!(
                    text.contains(&format!("-{flag}")) || text.contains(&format!("Fl {flag}")),
                    "{name} does not list -{flag} among the unsupported options"
                );
            }
            // Sockets, FIFOs and device nodes are rejected where cp -R
            // recreates them; an undocumented exit 1 is a nasty surprise.
            for word in ["socket", "FIFO", "device node"] {
                assert!(
                    text.to_lowercase().contains(&word.to_lowercase()),
                    "{name} does not document the {word} limitation"
                );
            }
            assert!(
                text.contains("Ownership") || text.contains("ownership"),
                "{name} does not say ownership is not preserved"
            );
        }
    }
}
