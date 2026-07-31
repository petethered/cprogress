use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cprogress");

/// Build a fixture covering the cases that differ between implementations:
/// nesting, symlinks, empty files, spaces in names, and a multi-chunk file.
fn make_fixture(root: &Path) {
    fs::create_dir_all(root.join("sub/deeper")).expect("mkdir");
    fs::write(root.join("hello.txt"), b"hello world").expect("write");
    fs::write(root.join("empty.txt"), b"").expect("write");
    fs::write(root.join("name with spaces.txt"), b"spaced").expect("write");
    fs::write(root.join("sub/nested.txt"), b"nested").expect("write");
    fs::write(root.join("sub/deeper/deep.bin"), vec![7u8; 3_000_000]).expect("write");
    symlink("hello.txt", root.join("link-to-file")).expect("symlink");
    symlink("sub", root.join("link-to-dir")).expect("symlink");
    fs::set_permissions(root.join("hello.txt"), fs::Permissions::from_mode(0o640))
        .expect("chmod");
    fs::set_permissions(root.join("sub"), fs::Permissions::from_mode(0o750)).expect("chmod");
}

/// Every path under `root`, relative, sorted, with type and mode.
fn describe(root: &Path) -> Vec<(String, String, u32)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).expect("read_dir");
        for entry in entries {
            let entry = entry.expect("entry");
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("lstat");
            let rel = path
                .strip_prefix(root)
                .expect("prefix")
                .to_string_lossy()
                .into_owned();
            let kind = if meta.file_type().is_symlink() {
                "symlink"
            } else if meta.is_dir() {
                "dir"
            } else {
                "file"
            };
            out.push((rel, kind.to_string(), meta.permissions().mode() & 0o777));
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Contents of every regular file, and targets of every symlink.
fn contents(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("lstat");
            let rel = path
                .strip_prefix(root)
                .expect("prefix")
                .to_string_lossy()
                .into_owned();
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).expect("readlink");
                out.push((rel, target.to_string_lossy().as_bytes().to_vec()));
            } else if meta.is_dir() {
                stack.push(path);
            } else {
                out.push((rel, fs::read(&path).expect("read")));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn recursive_force_copy_matches_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("src");
    make_fixture(&src);

    let ours = td.path().join("ours");
    let theirs = td.path().join("theirs");

    let a = Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&ours)
        .output()
        .expect("run cprogress");
    let b = Command::new("/bin/cp")
        .args(["-Rf"])
        .arg(&src)
        .arg(&theirs)
        .output()
        .expect("run cp");

    assert!(a.status.success(), "cprogress failed: {a:?}");
    assert!(b.status.success(), "cp failed: {b:?}");
    assert_eq!(describe(&ours), describe(&theirs), "tree structure differs");
    assert_eq!(contents(&ours), contents(&theirs), "file contents differ");
}

#[test]
fn copy_into_existing_directory_matches_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("src");
    make_fixture(&src);

    let ours = td.path().join("ours");
    let theirs = td.path().join("theirs");
    fs::create_dir(&ours).expect("mkdir");
    fs::create_dir(&theirs).expect("mkdir");

    Command::new(BIN)
        .args(["-Rf"])
        .arg(&src)
        .arg(&ours)
        .status()
        .expect("run cprogress");
    Command::new("/bin/cp")
        .args(["-Rf"])
        .arg(&src)
        .arg(&theirs)
        .status()
        .expect("run cp");

    assert_eq!(describe(&ours), describe(&theirs));
    assert_eq!(contents(&ours), contents(&theirs));
}

#[test]
fn multiple_sources_into_a_directory_match_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let a = td.path().join("a.txt");
    let b = td.path().join("b.txt");
    fs::write(&a, b"aaa").expect("write");
    fs::write(&b, b"bbb").expect("write");

    let ours = td.path().join("ours");
    let theirs = td.path().join("theirs");
    fs::create_dir(&ours).expect("mkdir");
    fs::create_dir(&theirs).expect("mkdir");

    Command::new(BIN)
        .arg(&a)
        .arg(&b)
        .arg(&ours)
        .status()
        .expect("run cprogress");
    Command::new("/bin/cp")
        .arg(&a)
        .arg(&b)
        .arg(&theirs)
        .status()
        .expect("run cp");

    assert_eq!(describe(&ours), describe(&theirs));
    assert_eq!(contents(&ours), contents(&theirs));
}

#[test]
fn overwriting_preserves_the_destination_mode_like_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("src.txt");
    fs::write(&src, b"fresh").expect("write");
    fs::set_permissions(&src, fs::Permissions::from_mode(0o666)).expect("chmod");

    let ours = td.path().join("ours.txt");
    let theirs = td.path().join("theirs.txt");
    for p in [&ours, &theirs] {
        fs::write(p, b"stale").expect("write");
        fs::set_permissions(p, fs::Permissions::from_mode(0o600)).expect("chmod");
    }

    Command::new(BIN).arg(&src).arg(&ours).status().expect("run");
    Command::new("/bin/cp").arg(&src).arg(&theirs).status().expect("run");

    let m1 = fs::metadata(&ours).expect("stat").permissions().mode() & 0o777;
    let m2 = fs::metadata(&theirs).expect("stat").permissions().mode() & 0o777;
    assert_eq!(m1, m2, "mode differs from cp");
    assert_eq!(fs::read(&ours).expect("read"), fs::read(&theirs).expect("read"));
}

#[test]
fn exit_codes_match_bin_cp() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("adir");
    fs::create_dir(&dir).expect("mkdir");
    let out = td.path().join("out");

    // A directory without -R.
    let ours = Command::new(BIN).arg(&dir).arg(&out).output().expect("run");
    let theirs = Command::new("/bin/cp").arg(&dir).arg(&out).output().expect("run");
    assert_eq!(ours.status.code(), theirs.status.code(), "dir-without-R code");

    // A missing source.
    let missing = td.path().join("nope");
    let ours = Command::new(BIN).arg(&missing).arg(&out).output().expect("run");
    let theirs = Command::new("/bin/cp").arg(&missing).arg(&out).output().expect("run");
    assert_eq!(ours.status.code(), theirs.status.code(), "missing-source code");

    // A usage error.
    let ours = Command::new(BIN).output().expect("run");
    let theirs = Command::new("/bin/cp").output().expect("run");
    assert_eq!(ours.status.code(), theirs.status.code(), "usage code");
}

#[test]
fn symlinks_without_recursive_are_followed_like_bin_cp() {
    // The existing symlink coverage only runs -Rf, where both implementations
    // preserve links. Without -R, cp dereferences, and we must too.
    let td = tempfile::tempdir().expect("tempdir");
    fs::write(td.path().join("f.txt"), b"hello").expect("write");
    fs::create_dir(td.path().join("dir")).expect("mkdir");
    symlink("f.txt", td.path().join("to-file")).expect("symlink");
    symlink("dir", td.path().join("to-dir")).expect("symlink");
    symlink("nowhere", td.path().join("broken")).expect("symlink");

    for name in ["to-file", "to-dir", "broken"] {
        let src = td.path().join(name);
        let ours = td.path().join(format!("ours-{name}"));
        let theirs = td.path().join(format!("theirs-{name}"));

        let a = Command::new(BIN).arg(&src).arg(&ours).output().expect("run cprogress");
        let b = Command::new("/bin/cp").arg(&src).arg(&theirs).output().expect("run cp");

        assert_eq!(a.status.code(), b.status.code(), "{name}: exit code differs");
        assert_eq!(
            String::from_utf8_lossy(&a.stderr).replace("cprogress: ", "cp: "),
            String::from_utf8_lossy(&b.stderr),
            "{name}: stderr differs"
        );

        let mine = fs::symlink_metadata(&ours).ok().map(|m| m.file_type().is_symlink());
        let reference = fs::symlink_metadata(&theirs).ok().map(|m| m.file_type().is_symlink());
        assert_eq!(mine, reference, "{name}: destination kind differs");
    }

    // Spelled out, so a regression cannot pass by matching cp on nothing:
    // the followed link is a regular file with the target's contents.
    let followed = td.path().join("ours-to-file");
    assert!(
        !fs::symlink_metadata(&followed).expect("lstat").file_type().is_symlink(),
        "the link was preserved instead of followed"
    );
    assert_eq!(fs::read(&followed).expect("read"), b"hello");
}

#[test]
fn an_io_error_reads_exactly_like_cps() {
    // Rust's io::Error Display appends "(os error 13)", which cp never
    // prints; the errno text has to be translated, not borrowed.
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("secret.txt");
    fs::write(&src, b"nope").expect("write");
    fs::set_permissions(&src, fs::Permissions::from_mode(0o000)).expect("chmod");

    let ours = Command::new(BIN)
        .arg(&src)
        .arg(td.path().join("ours"))
        .output()
        .expect("run cprogress");
    let theirs = Command::new("/bin/cp")
        .arg(&src)
        .arg(td.path().join("theirs"))
        .output()
        .expect("run cp");

    let mine = String::from_utf8_lossy(&ours.stderr).replace("cprogress: ", "cp: ");
    let reference = String::from_utf8_lossy(&theirs.stderr).into_owned();

    assert_eq!(mine, reference, "error text differs from cp");
    assert!(
        reference.contains("Permission denied"),
        "test assumption broken, cp said: {reference:?}"
    );
    assert!(
        !mine.contains("os error"),
        "the raw errno leaked into the message: {mine:?}"
    );
    assert_eq!(ours.status.code(), theirs.status.code());
}

#[test]
fn bare_invocation_prints_cps_usage_on_stderr() {
    let ours = Command::new(BIN).output().expect("run");
    let theirs = Command::new("/bin/cp").output().expect("run");

    let mine = String::from_utf8_lossy(&ours.stderr).into_owned();
    // A single replace suffices: every "usage: cp " line becomes
    // "usage: cprogress " because "cp " already matches within it.
    let reference = String::from_utf8_lossy(&theirs.stderr).replace("cp ", "cprogress ");

    assert_eq!(mine.trim_end(), reference.trim_end(), "usage text differs");
    assert!(ours.stdout.is_empty(), "usage should not go to stdout");
}

#[test]
fn help_prints_a_manual_on_stdout_and_succeeds() {
    for flag in ["-h", "--help"] {
        let out = Command::new(BIN).arg(flag).output().expect("run");
        assert!(out.status.success(), "{flag} should exit 0");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("NAME"), "{flag}: no NAME section");
        assert!(text.contains("SYNOPSIS"), "{flag}: no SYNOPSIS section");
        assert!(text.contains("EXIT STATUS"), "{flag}: no EXIT STATUS section");
        assert!(
            text.contains("-p -i -n"),
            "{flag}: unsupported options not listed"
        );
        assert!(out.stderr.is_empty(), "{flag}: help should not use stderr");
    }
}
