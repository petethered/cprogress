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
