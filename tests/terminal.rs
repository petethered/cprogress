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
fn the_bar_uses_the_terminal_while_verbose_lines_go_to_stdout() {
    // The unit tests cover Bar's two sinks in isolation. This pins the wiring
    // end to end: stderr is a real terminal (so the bar draws), stdout is a
    // pipe. cp -v writes to stdout, so every -v line must land in the pipe and
    // none of them on the terminal.
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("s");
    fs::create_dir(&src).expect("mkdir");
    for i in 0..3 {
        fs::write(src.join(format!("f{i}.bin")), vec![0u8; 9 * 1024 * 1024]).expect("write");
    }
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

    let mut child = Command::new(BIN)
        .args(["-Rv"])
        .arg(&src)
        .arg(&dst)
        .stdout(Stdio::piped())
        .stderr(unsafe { Stdio::from_raw_fd(slave) })
        .spawn()
        .expect("spawn");

    // Drain the pty before waiting; see the note in draws_a_bar_on_a_pty.
    let mut reader = unsafe { fs::File::from_raw_fd(master) };
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    let terminal = String::from_utf8_lossy(&buf).into_owned();

    let mut stdout_text = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout_text);
    }
    let status = child.wait().expect("wait");
    assert!(status.success());

    // The bar drew on the terminal.
    assert!(terminal.contains("total"), "no aggregate bar on the tty: {terminal:?}");
    // Every -v line landed on stdout, one per op (3 files + the directory).
    let arrows = stdout_text.lines().filter(|l| l.contains(" -> ")).count();
    assert_eq!(arrows, 4, "expected 4 -v lines on stdout, got {stdout_text:?}");
    // And none of them leaked onto the terminal.
    assert!(
        !terminal.contains(" -> "),
        "a -v line was drawn on the bar's stream: {terminal:?}"
    );
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

    let mut child = Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&dst)
        .stderr(unsafe { Stdio::from_raw_fd(slave) })
        .spawn()
        .expect("spawn");

    // Read before waiting on the child, not after: on macOS, once every
    // reference to the slave side is closed (which `child.wait()` would
    // leave as the case, since the child holds the only other one), the
    // pty's queued canonical-mode output is dropped rather than delivered
    // on the next read — `poll` reports `POLLIN|POLLHUP` but `read` then
    // returns 0. Draining `master` first, while the child's copy of the
    // slave fd is still open, avoids that race; `read_to_end` returns once
    // the child exits and its fd closes, which is also what keeps this from
    // hanging.
    let mut reader = unsafe { fs::File::from_raw_fd(master) };
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    let out = String::from_utf8_lossy(&buf);

    let status = child.wait().expect("wait");
    assert!(status.success());

    assert!(out.contains("total"), "no aggregate bar: {out:?}");
    assert!(out.contains("big.bin"), "no file line: {out:?}");
    assert!(out.contains("copied in"), "no summary: {out:?}");
}
