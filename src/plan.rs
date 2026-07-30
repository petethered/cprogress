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
///
/// `dst` usually does not exist yet, so it cannot be canonicalized directly.
/// Walk up from `dst` to the nearest ancestor that does exist (stopping at
/// most at the filesystem root, which always does), canonicalize that
/// ancestor, then reattach the stripped-off components. This handles a
/// destination several directories deep with none of them created yet, not
/// just an immediate parent — a single-level fallback (checking only
/// `dst.parent()`) misses `s` vs `s/a/b` when neither `a` nor `b` exists.
fn dest_inside_source(src: &Path, dst: &Path) -> bool {
    let Ok(src_c) = src.canonicalize() else {
        return false;
    };

    let mut existing = dst;
    let mut suffix: Vec<&std::ffi::OsStr> = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                suffix.push(name);
                existing = parent;
            }
            _ => break,
        }
    }

    let Ok(mut dst_c) = existing.canonicalize() else {
        return false;
    };
    for name in suffix.into_iter().rev() {
        dst_c = dst_c.join(name);
    }

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

#[cfg(test)]
mod dest_inside_source_tests {
    use super::*;

    #[test]
    fn detects_nesting_several_nonexistent_levels_deep() {
        // `s/a/b` where neither `a` nor `b` exists yet: a fallback that only
        // checks `dst.parent()` misses this, since that parent (`s/a`) also
        // doesn't exist and its canonicalize() fails.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        let dst = src.join("a").join("b");
        assert!(
            dest_inside_source(&src, &dst),
            "should detect nesting two levels deep"
        );
    }

    #[test]
    fn sibling_destination_is_not_flagged_as_nested() {
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir");
        let dst = td.path().join("other").join("deeper");
        assert!(!dest_inside_source(&src, &dst));
    }
}
