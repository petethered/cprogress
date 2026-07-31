use crate::cli::Options;
use crate::msg;
use crate::plan::{CopyOp, Plan};
use crate::progress::Progress;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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
    /// Destinations whose `mkdir` failed. `cp` recurses as it copies, so a
    /// failed directory ends that descent and yields exactly one error. This
    /// op list is flat, so anything landing under one of these is skipped
    /// instead of restating the same cause once per descendant — which on a
    /// large tree would bury the real failure under thousands of ENOENTs.
    failed_dirs: Vec<PathBuf>,
}

impl Runner<'_> {
    fn fail(&mut self, path: &Path, err: &std::io::Error) {
        self.progress.note(&msg::io_error(path, err));
        self.outcome.failed = true;
    }

    /// Whether `dst` lies under a directory whose creation already failed.
    /// `starts_with` compares whole path components, so `dest/ab` is not
    /// pruned by a failure at `dest/a`.
    fn pruned(&self, dst: &Path) -> bool {
        self.failed_dirs.iter().any(|dir| dst.starts_with(dir))
    }

    /// `-v` lines go to stdout via `verbose`, not to `note`: `cp` prints them
    /// there, and a script redirecting stdout must capture them.
    fn verbose(&mut self, src: &Path, dst: &Path) {
        if self.opts.verbose {
            self.progress
                .verbose(&format!("{} -> {}", src.display(), dst.display()));
        }
    }

    fn aborted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    /// Returns whether the directory ended up in place, so callers can gate
    /// `-v` on it: reporting `src -> dst` for a directory that was never
    /// created would be actively misleading.
    fn create_dir(&mut self, dst: &Path, mode: u32) -> bool {
        match fs::create_dir(dst) {
            Ok(()) => {
                // Only newly created directories take the source mode. A
                // failure here is soft — the directory itself exists — so it
                // still counts as created, matching copy_file's own handling
                // of a post-copy mode-setting failure.
                let perms = fs::Permissions::from_mode(mode & !self.mask);
                if let Err(e) = fs::set_permissions(dst, perms) {
                    self.fail(dst, &e);
                }
                true
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => true,
            Err(e) => {
                self.fail(dst, &e);
                // Nothing below this can succeed, and cp would never have
                // tried, so stop reporting the consequences.
                self.failed_dirs.push(dst.to_path_buf());
                false
            }
        }
    }

    /// Returns whether the symlink ended up in place; see `create_dir`.
    fn symlink(&mut self, target: &Path, dst: &Path) -> bool {
        let result = std::os::unix::fs::symlink(target, dst);
        match result {
            Ok(()) => true,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Replacing an existing entry is what makes repeated runs work.
                if fs::remove_file(dst).is_ok() {
                    match std::os::unix::fs::symlink(target, dst) {
                        Ok(()) => true,
                        Err(e2) => {
                            self.fail(dst, &e2);
                            false
                        }
                    }
                } else {
                    self.fail(dst, &e);
                    false
                }
            }
            Err(e) => {
                self.fail(dst, &e);
                false
            }
        }
    }

    /// Open the destination, retrying after unlinking when `-f` is in effect
    /// *and* the failure indicates dst itself is the obstacle (a read-only
    /// destination, the case `-f` exists for). Any other error — e.g. EMFILE,
    /// a process-wide, transient condition with nothing to do with dst —
    /// propagates untouched, so a fluke never costs the user their file.
    fn open_dest(&mut self, dst: &Path) -> std::io::Result<(File, bool)> {
        let existed = dst.symlink_metadata().is_ok();
        let attempt = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(dst);
        match attempt {
            Ok(f) => Ok((f, existed)),
            Err(e) if self.opts.force && e.kind() == ErrorKind::PermissionDenied => {
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

    /// Report a failure part-way through a file and credit the aggregate with
    /// the bytes that will now never be copied, so the bar still reaches 100%.
    /// Every abandoned path through `copy_file` goes through here, so none of
    /// them can forget the second half.
    fn fail_remaining(&mut self, path: &Path, err: &std::io::Error, remaining: u64) {
        self.fail(path, err);
        self.progress.advance(remaining);
    }

    fn copy_file(&mut self, src: &Path, dst: &Path, size: u64, mode: u32, display: &Path) {
        self.progress.file_started(display, size);

        // Declared before the first fallible step so every failure path can
        // credit the aggregate with the same expression.
        let mut copied: u64 = 0;

        let mut input = match File::open(src) {
            Ok(f) => f,
            Err(e) => {
                self.fail_remaining(src, &e, size.saturating_sub(copied));
                return;
            }
        };

        let (mut output, existed) = match self.open_dest(dst) {
            Ok(pair) => pair,
            Err(e) => {
                self.fail_remaining(dst, &e, size.saturating_sub(copied));
                return;
            }
        };

        let mut buf = vec![0u8; BUFFER_SIZE];
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
                    self.fail_remaining(src, &e, size.saturating_sub(copied));
                    return;
                }
            };
            // write_all loops internally, so short writes are handled.
            if let Err(e) = output.write_all(&buf[..n]) {
                self.fail_remaining(dst, &e, size.saturating_sub(copied));
                return;
            }
            copied = copied.saturating_add(n as u64);
            self.progress.advance(n as u64);
        }

        if let Err(e) = output.flush() {
            self.fail_remaining(dst, &e, size.saturating_sub(copied));
            return;
        }

        // A file whose size changed under us should still land the bar on 100%.
        if copied < size {
            self.progress.advance(size - copied);
        }

        // `mode` was captured during the scan, from the metadata the walk
        // already held. Re-stat'ing here would cost a stat per file and reopen
        // the very TOCTOU window plan-then-execute exists to close.
        if !existed {
            let perms = fs::Permissions::from_mode(mode & !self.mask);
            if let Err(e) = fs::set_permissions(dst, perms) {
                self.fail(dst, &e);
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
        failed_dirs: Vec::new(),
    };

    for op in &plan.ops {
        if runner.aborted() {
            runner.outcome.interrupted = true;
            break;
        }

        let target = match op {
            CopyOp::CreateDir { dst, .. }
            | CopyOp::Symlink { dst, .. }
            | CopyOp::CopyFile { dst, .. } => dst,
        };
        if runner.pruned(target) {
            // The cause was reported at the directory that failed; saying it
            // again for each descendant is noise cp never makes. Still credit
            // the aggregate, as every other failure does.
            if let CopyOp::CopyFile { size, .. } = op {
                runner.progress.advance(*size);
            }
            continue;
        }

        match op {
            CopyOp::CreateDir { src, dst, mode } => {
                if runner.create_dir(dst, *mode) {
                    runner.verbose(src, dst);
                }
            }
            CopyOp::Symlink { src, target, dst } => {
                if runner.symlink(target, dst) {
                    runner.verbose(src, dst);
                }
            }
            CopyOp::CopyFile { src, dst, size, mode, display } => {
                runner.copy_file(src, dst, *size, *mode, display)
            }
        }
    }

    runner.outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan;
    use crate::progress::Null;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn run_plan(p: &Plan, o: &Options) -> Outcome {
        let mut sink: Vec<u8> = Vec::new();
        let mut prog = Null::new(&mut sink, std::io::sink());
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

    /// Records everything the engine reports, so a test can assert on any of
    /// it without a bespoke Progress impl per case.
    #[derive(Default)]
    struct Spy {
        notes: Vec<String>,
        verbose: Vec<String>,
        advanced: u64,
        files: Vec<PathBuf>,
    }

    impl crate::progress::Progress for Spy {
        fn scan_tick(&mut self, _f: usize, _b: u64) {}
        fn start(&mut self, _t: u64, _c: usize) {}
        fn file_started(&mut self, p: &Path, _s: u64) {
            self.files.push(p.to_path_buf());
        }
        fn advance(&mut self, bytes: u64) {
            self.advanced = self.advanced.saturating_add(bytes);
        }
        fn note(&mut self, l: &str) {
            self.notes.push(l.to_string());
        }
        fn verbose(&mut self, l: &str) {
            self.verbose.push(l.to_string());
        }
        fn finish(&mut self) {}
    }

    fn spy_run(p: &Plan, o: &Options) -> (Spy, Outcome) {
        let mut spy = Spy::default();
        let flag = AtomicBool::new(false);
        let outcome = run(p, o, &mut spy, &flag);
        (spy, outcome)
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
            let mut prog = Null::new(&mut sink, std::io::sink());
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

        let (spy, _) = spy_run(&p, &recursive());

        assert_eq!(spy.advanced, total, "aggregate did not reach the total");
    }

    #[test]
    fn a_failed_directory_reports_once_and_prunes_its_whole_subtree() {
        // Verified against /bin/cp: `cp -R a d` with d unwritable prints
        // exactly one line, "cp: d/a: Permission denied". cp recurses during
        // the copy, so a failed mkdir ends that descent. Our op list is flat,
        // so without pruning every descendant adds a spurious ENOENT and
        // buries the real cause.
        // Not root: root ignores the write-permission bit this relies on.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("a");
        fs::create_dir_all(src.join("b/c/d")).expect("mkdir");
        fs::write(src.join("x"), b"1").expect("write");
        fs::write(src.join("b/y"), b"22").expect("write");
        fs::write(src.join("b/c/z"), b"333").expect("write");
        fs::write(src.join("b/c/d/w"), b"4444").expect("write");
        let dst = td.path().join("dest");
        fs::create_dir(&dst).expect("mkdir");
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o555)).expect("chmod");

        let p = plan_for(&[src], &dst, &recursive());
        let total = p.total_bytes;
        let (spy, outcome) = spy_run(&p, &recursive());
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o755)).expect("restore chmod");

        assert!(outcome.failed, "expected the directory creation to fail");
        assert_eq!(
            spy.notes.len(),
            1,
            "one failed mkdir should produce one error, got {:?}",
            spy.notes
        );
        assert!(
            spy.notes[0].contains("Permission denied"),
            "wrong error: {:?}",
            spy.notes[0]
        );
        // Skipped ops still credit the aggregate, as other failures do, or the
        // bar sticks below 100% forever.
        assert_eq!(spy.advanced, total, "aggregate did not reach the total");
    }

    #[test]
    fn a_failed_directory_does_not_prune_a_sibling_with_a_shared_name_prefix() {
        // `dest/ab` must not be swallowed by a failure at `dest/a`: the check
        // is on path components, not on string prefixes.
        let td = tempfile::tempdir().expect("tempdir");
        let a = td.path().join("a");
        fs::create_dir(&a).expect("mkdir");
        fs::write(a.join("f"), b"1").expect("write");
        let ab = td.path().join("ab");
        fs::create_dir(&ab).expect("mkdir");
        fs::write(ab.join("g"), b"2").expect("write");
        let dst = td.path().join("dest");
        fs::create_dir(&dst).expect("mkdir");

        let p = plan_for(&[a, ab], &dst, &recursive());
        // Make only `dest/a` fail, by putting a file where its directory goes.
        fs::write(dst.join("a"), b"blocker").expect("write");

        let (spy, outcome) = spy_run(&p, &recursive());

        assert!(outcome.failed);
        assert_eq!(
            fs::read(dst.join("ab/g")).expect("sibling should have been copied"),
            b"2"
        );
        assert!(
            spy.notes.iter().all(|n| !n.contains("ab")),
            "the sibling was pruned: {:?}",
            spy.notes
        );
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
        let p = plan_for(&[src], &dst, &verbose);
        let mut notes: Vec<u8> = Vec::new();
        let mut lines: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        {
            let mut prog = Null::new(&mut notes, &mut lines);
            run(&p, &verbose, &mut prog, &flag);
        }
        let out = String::from_utf8(lines).expect("utf8");
        assert!(out.contains("->"), "got {out:?}");
        assert!(out.contains("a"), "got {out:?}");
        // cp -v writes to stdout; the error stream stays clean on a good copy.
        assert!(notes.is_empty(), "a -v line reached the error stream: {notes:?}");
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
        let p = plan_for(std::slice::from_ref(&src), &dst, &verbose);
        let mut lines: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        {
            let mut prog = Null::new(std::io::sink(), &mut lines);
            run(&p, &verbose, &mut prog, &flag);
        }
        let out = String::from_utf8(lines).expect("utf8");

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
    fn progress_file_line_gets_a_root_relative_path_not_the_absolute_source() {
        // The progress bar's per-file line must show a path relative to the
        // source root (e.g. "s/sub/g.txt"), not the full absolute source
        // path — otherwise a long tempdir/absolute prefix crowds out the
        // useful tail on a real terminal. `-v` is unaffected: it keeps using
        // the full paths (see verbose_reports_each_copy and
        // verbose_names_the_real_source_for_directories_and_symlinks).
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir_all(src.join("sub")).expect("mkdir");
        fs::write(src.join("sub/g.txt"), b"hi").expect("write");
        let dst = td.path().join("d");

        let p = plan_for(std::slice::from_ref(&src), &dst, &recursive());
        let (spy, _) = spy_run(&p, &recursive());

        assert_eq!(spy.files, vec![PathBuf::from("s/sub/g.txt")]);
        assert!(
            !spy.files[0].starts_with(td.path()),
            "file line still carries the absolute prefix: {:?}",
            spy.files[0]
        );
    }

    #[test]
    fn verbose_does_not_report_a_failed_directory_creation() {
        // Not root: root ignores the write-permission bit this test relies on
        // to make `create_dir` genuinely fail (rather than the read/scan step).
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("s");
        fs::create_dir(&src).expect("mkdir src");
        let dst = td.path().join("d");
        fs::create_dir(&dst).expect("mkdir dst");
        // dst exists as a directory, so the plan copies `s` *into* it as
        // `d/s` — creating that entry needs write permission on `d`, which
        // this deliberately withholds.
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o555)).expect("chmod");

        let verbose = Options {
            recursive: true,
            verbose: true,
            ..Options::default()
        };
        let p = plan_for(&[src], &dst, &verbose);
        let mut notes: Vec<u8> = Vec::new();
        let mut lines: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        let outcome = {
            let mut prog = Null::new(&mut notes, &mut lines);
            run(&p, &verbose, &mut prog, &flag)
        };
        // Restore so tempdir's own cleanup can remove `dst`.
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o755)).expect("restore chmod");

        assert!(outcome.failed, "expected the directory creation to fail");
        let notes = String::from_utf8(notes).expect("utf8");
        assert!(
            notes.lines().any(|l| l.contains("Permission denied")),
            "expected a permission-denied failure note: {notes:?}"
        );
        // Asserted on the -v stream specifically, which is the only place a
        // "src -> dst" line can now appear.
        assert!(
            lines.is_empty(),
            "verbose reported a directory creation that actually failed: {:?}",
            String::from_utf8_lossy(&lines)
        );
    }

    #[test]
    fn verbose_does_not_report_a_failed_symlink_creation() {
        // Not root: root ignores the write-permission bit this test relies on.
        let td = tempfile::tempdir().expect("tempdir");
        let src = td.path().join("link");
        symlink("nowhere", &src).expect("symlink src");
        let dst = td.path().join("d");
        fs::create_dir(&dst).expect("mkdir dst");
        // dst is an existing directory, so the plan targets `d/link` —
        // creating that entry needs write permission on `d`.
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o555)).expect("chmod");

        // -R is required to get a Symlink op at all: without it the scan
        // dereferences the link, as cp does, and this dangling one would be
        // reported as a missing source instead.
        let verbose = Options {
            recursive: true,
            verbose: true,
            ..Options::default()
        };
        let p = plan_for(&[src], &dst, &verbose);
        let mut notes: Vec<u8> = Vec::new();
        let mut lines: Vec<u8> = Vec::new();
        let flag = AtomicBool::new(false);
        let outcome = {
            let mut prog = Null::new(&mut notes, &mut lines);
            run(&p, &verbose, &mut prog, &flag)
        };
        fs::set_permissions(&dst, fs::Permissions::from_mode(0o755)).expect("restore chmod");

        assert!(outcome.failed, "expected the symlink creation to fail");
        let notes = String::from_utf8(notes).expect("utf8");
        assert!(
            notes.lines().any(|l| l.contains("Permission denied")),
            "expected a permission-denied failure note: {notes:?}"
        );
        assert!(
            lines.is_empty(),
            "verbose reported a symlink creation that actually failed: {:?}",
            String::from_utf8_lossy(&lines)
        );
    }

    #[test]
    fn force_open_dest_does_not_unlink_on_a_non_permission_error() {
        // Distinguishes "dst itself is the obstacle" (EACCES / PermissionDenied,
        // which -f should treat by unlinking dst and retrying) from "the open
        // failed for a reason unrelated to dst" — here ELOOP, from a
        // self-referential symlink. The latter must propagate without ever
        // touching dst: unlinking it would destroy whatever was there over an
        // error that had nothing to do with dst's own writability (the same
        // class of bug as EMFILE, which is not reproducible portably/safely
        // without mutating the whole process's fd limit).
        let td = tempfile::tempdir().expect("tempdir");
        let dst = td.path().join("loop");
        symlink("loop", &dst).expect("create a self-referential symlink");

        let forced = Options {
            force: true,
            ..Options::default()
        };
        let mut sink: Vec<u8> = Vec::new();
        let mut prog = Null::new(&mut sink, std::io::sink());
        let flag = AtomicBool::new(false);
        let mut runner = Runner {
            opts: &forced,
            progress: &mut prog,
            interrupted: &flag,
            mask: umask(),
            outcome: Outcome::default(),
            failed_dirs: Vec::new(),
        };

        let result = runner.open_dest(&dst);

        let kind = result.as_ref().err().map(|e| e.kind());
        assert_ne!(
            kind,
            Some(ErrorKind::PermissionDenied),
            "test assumption broken: a symlink loop should fail with something \
             other than PermissionDenied, got {kind:?}"
        );
        assert!(result.is_err(), "opening a symlink loop should fail");
        let meta = fs::symlink_metadata(&dst).expect("dst must still exist");
        assert!(
            meta.file_type().is_symlink(),
            "dst must survive untouched: still the original symlink"
        );
        assert_eq!(
            fs::read_link(&dst).expect("readlink"),
            Path::new("loop"),
            "dst's target must be unchanged"
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
            let mut prog = Null::new(&mut sink, std::io::sink());
            run(&p, &recursive(), &mut prog, &flag)
        };

        assert!(outcome.interrupted);
    }
}
