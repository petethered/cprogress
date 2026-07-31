use crate::cli::Options;
use crate::msg::{self, PROGRAM};
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Every variant carries `src` so `-v` can report `src -> dst` for directories
/// and symlinks, not just regular files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOp {
    CreateDir { src: PathBuf, dst: PathBuf, mode: u32 },
    /// `display` is `src` shortened to be relative to the source root's
    /// parent directory (falling back to `src` itself when a relative form
    /// cannot be derived). It exists only for the progress bar's per-file
    /// line, which would otherwise be dominated by a long absolute prefix;
    /// `-v` and error messages keep using `src`/`dst` in full.
    CopyFile { src: PathBuf, dst: PathBuf, size: u64, display: PathBuf },
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
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::DestNotDirectory(p) => {
                write!(f, "{PROGRAM}: {}: Not a directory", p.display())
            }
        }
    }
}

/// True when `a` and `b` resolve to the same file: same device and inode.
/// This is what `cp` actually rejects ("are identical (not copied)."), not
/// path nesting — a hard link to the same file has a different path but the
/// same identity, which a string or canonicalized-path comparison would miss.
/// Symlinks are followed (`fs::metadata`), matching `cp`'s own stat-based
/// check. If either side cannot be stat'd, this reports no match rather than
/// failing the whole plan over it.
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(am), Ok(bm)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    am.dev() == bm.dev() && am.ino() == bm.ino()
}

/// Compute a short display path for `src`, relative to `root_parent` (the
/// parent directory of the source argument the user passed on the command
/// line). Falls back to the full `src` when it cannot be made relative —
/// e.g. `root_parent` is empty or unrelated — since a wrong-but-harmless
/// fallback beats failing the whole plan over a cosmetic label.
fn display_path(src: &Path, root_parent: &Path) -> PathBuf {
    match src.strip_prefix(root_parent) {
        Ok(rel) if rel.as_os_str().is_empty() => src.to_path_buf(),
        Ok(rel) => rel.to_path_buf(),
        Err(_) => src.to_path_buf(),
    }
}

struct Scanner<'a> {
    plan: Plan,
    opts: &'a Options,
    tick: &'a mut dyn FnMut(usize, u64),
}

impl Scanner<'_> {
    fn note_io(&mut self, path: &Path, err: &io::Error) {
        self.plan.errors.push(msg::io_error(path, err));
    }

    fn add_file(&mut self, src: PathBuf, dst: PathBuf, size: u64, root_parent: &Path) {
        let display = display_path(&src, root_parent);
        self.plan.ops.push(CopyOp::CopyFile { src, dst, size, display });
        self.plan.total_bytes = self.plan.total_bytes.saturating_add(size);
        self.plan.file_count += 1;
        (self.tick)(self.plan.file_count, self.plan.total_bytes);
    }

    /// Add `src` at `dst`, recursing if it is a directory. `root_parent` is
    /// the parent directory of the top-level source this call descends from;
    /// it stays fixed across the recursion so every file's display path is
    /// relative to the same root, however deep it is nested.
    fn visit(&mut self, src: &Path, dst: &Path, top_level: bool, root_parent: &Path) {
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
                    "{PROGRAM}: {} is a directory (not copied).",
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
                self.visit(&src.join(&name), &dst.join(&name), false, root_parent);
            }
            return;
        }

        if meta.is_file() {
            self.add_file(src.to_path_buf(), dst.to_path_buf(), meta.len(), root_parent);
            return;
        }

        // Sockets, fifos, devices: out of scope, reported rather than skipped
        // silently so the exit status reflects that something was left behind.
        let _ = top_level;
        self.plan.errors.push(format!(
            "{PROGRAM}: {}: unsupported file type (not copied)",
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

    let mut scanner = Scanner {
        plan: Plan::default(),
        opts,
        tick,
    };

    for src in sources {
        let target = destination_for(src, dest, dest_is_dir);

        // What real `cp` actually rejects is copying a file onto itself, not
        // a destination nested inside a source directory (see module docs
        // on `same_file`). Skip this source without emitting any op for it;
        // the run continues with the rest.
        if same_file(src, &target) {
            scanner.plan.errors.push(format!(
                "{PROGRAM}: {} and {} are identical (not copied).",
                src.display(),
                target.display()
            ));
            continue;
        }

        let root_parent = src.parent().unwrap_or(Path::new(""));
        scanner.visit(src, &target, true, root_parent);
    }

    Ok(scanner.plan)
}

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

        let plan = build_in(std::slice::from_ref(&src), &dst, &opts(false)).expect("plan");
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
    fn destination_nested_in_source_is_allowed_like_cp() {
        // Verified against /bin/cp: `cp -R s s/inner` succeeds (exit 0),
        // producing s/inner/f.txt. cp snapshots the directory listing before
        // copying, so `inner` is never re-visited — and so do we, since
        // plan::build finishes the whole scan before engine::run creates
        // anything. This used to be rejected as DestInsideSource, which was
        // a spec defect: cp only rejects a source and destination resolving
        // to the *same file* (see `same_file_source_and_dest_is_reported`).
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        fs::write(src.join("f.txt"), b"hi").expect("write");
        let dst = src.join("inner");

        let plan =
            build_in(std::slice::from_ref(&src), &dst, &opts(true)).expect("plan should succeed");
        assert!(plan.errors.is_empty(), "got errors: {:?}", plan.errors);
        assert_eq!(plan.file_count, 1);
        assert!(plan
            .ops
            .iter()
            .any(|o| matches!(o, CopyOp::CopyFile { dst, .. } if dst.ends_with("inner/f.txt"))));
    }

    #[test]
    fn same_file_source_and_dest_is_reported_and_produces_no_op() {
        // Matches `cp a.txt a.txt`: "cp: a.txt and a.txt are identical (not
        // copied)." and exit 1. This is non-fatal at the plan level: it is
        // recorded in `errors`, not returned as a `PlanError`.
        let td = tempfile::tempdir().expect("tempdir");
        let a = td.path().join("a.txt");
        fs::write(&a, b"x").expect("write");

        let plan = build_in(std::slice::from_ref(&a), &a, &opts(false)).expect("plan");
        assert!(plan.ops.is_empty());
        assert_eq!(plan.total_bytes, 0);
        assert_eq!(plan.errors.len(), 1);
        assert_eq!(
            plan.errors[0],
            format!(
                "cprogress: {} and {} are identical (not copied).",
                a.display(),
                a.display()
            )
        );
    }

    #[test]
    fn a_hard_link_to_the_same_inode_is_reported_as_identical() {
        // Same underlying file via two different names: a path/string
        // comparison would miss this, but dev+inode comparison catches it.
        let td = tempfile::tempdir().expect("tempdir");
        let a = td.path().join("a.txt");
        fs::write(&a, b"x").expect("write");
        let link = td.path().join("link.txt");
        fs::hard_link(&a, &link).expect("hard_link");

        let plan = build_in(&[a], &link, &opts(false)).expect("plan");
        assert!(plan.ops.is_empty());
        assert_eq!(plan.errors.len(), 1);
        assert!(
            plan.errors[0].contains("identical (not copied)"),
            "got {:?}",
            plan.errors[0]
        );
    }

    #[test]
    fn two_files_with_identical_content_but_different_inodes_are_not_flagged() {
        let td = tempfile::tempdir().expect("tempdir");
        let a = td.path().join("a.txt");
        let b = td.path().join("b.txt");
        fs::write(&a, b"same content").expect("write");
        fs::write(&b, b"same content").expect("write");

        let plan = build_in(&[a], &b, &opts(false)).expect("plan");
        assert!(plan.errors.is_empty(), "got errors: {:?}", plan.errors);
        assert_eq!(plan.file_count, 1);
    }

    #[test]
    fn a_nested_files_display_path_is_relative_to_the_source_root() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir_all(src.join("sub")).expect("mkdir");
        fs::write(src.join("sub/g.txt"), b"123").expect("write");
        let dst = td.path().join("d");

        let plan = build_in(&[src], &dst, &opts(true)).expect("plan");
        let display = plan
            .ops
            .iter()
            .find_map(|o| match o {
                CopyOp::CopyFile { dst, display, .. } if dst.ends_with("g.txt") => {
                    Some(display.clone())
                }
                _ => None,
            })
            .expect("file op");
        assert_eq!(display, Path::new("s/sub/g.txt"));
    }

    #[test]
    fn a_top_level_files_display_path_has_no_absolute_prefix() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a.txt");
        fs::write(&src, b"hello").expect("write");
        let dst = td.path().join("b.txt");

        let plan = build_in(&[src], &dst, &opts(false)).expect("plan");
        match &plan.ops[0] {
            CopyOp::CopyFile { display, .. } => assert_eq!(display, Path::new("a.txt")),
            other => panic!("wrong op: {other:?}"),
        }
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
