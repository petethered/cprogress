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

    fn copy_file(&mut self, src: &Path, dst: &Path, size: u64, display: &Path) {
        self.progress.file_started(display, size);

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
            CopyOp::CopyFile { src, dst, size, display } => {
                runner.copy_file(src, dst, *size, display)
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
        let p = plan_for(&[src], &dst, &verbose);
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
        let p = plan_for(std::slice::from_ref(&src), &dst, &verbose);
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

        struct Recorder {
            seen: Vec<PathBuf>,
        }
        impl crate::progress::Progress for Recorder {
            fn scan_tick(&mut self, _f: usize, _b: u64) {}
            fn start(&mut self, _t: u64, _c: usize) {}
            fn file_started(&mut self, p: &Path, _s: u64) {
                self.seen.push(p.to_path_buf());
            }
            fn advance(&mut self, _bytes: u64) {}
            fn note(&mut self, _l: &str) {}
            fn finish(&mut self) {}
        }

        let p = plan_for(std::slice::from_ref(&src), &dst, &recursive());
        let mut recorder = Recorder { seen: Vec::new() };
        let flag = AtomicBool::new(false);
        run(&p, &recursive(), &mut recorder, &flag);

        assert_eq!(recorder.seen, vec![PathBuf::from("s/sub/g.txt")]);
        assert!(
            !recorder.seen[0].starts_with(td.path()),
            "file line still carries the absolute prefix: {:?}",
            recorder.seen[0]
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
