use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_cprogress");

/// Ignored by default: needs several GB of free disk and real wall-clock time.
/// Run with `cargo test --test large_file -- --ignored --nocapture`.
#[test]
#[ignore]
fn copies_a_file_larger_than_four_gibibytes() {
    let td = tempfile::tempdir().expect("tempdir");
    let src = td.path().join("huge.bin");
    let dst = td.path().join("copy.bin");

    let size: u64 = 5 * 1024 * 1024 * 1024; // 5 GiB, past the 32-bit boundary
    {
        let mut f = File::create(&src).expect("create");
        // Sparse body with real data at both ends, so the copy must traverse
        // the whole length without materializing it in memory.
        f.write_all(b"START").expect("write head");
        f.seek(SeekFrom::Start(size - 3)).expect("seek");
        f.write_all(b"END").expect("write tail");
        f.flush().expect("flush");
    }
    assert_eq!(fs::metadata(&src).expect("stat").len(), size);

    let status = Command::new(BIN)
        .arg(&src)
        .arg(&dst)
        .status()
        .expect("run cprogress");
    assert!(status.success());

    let copied = fs::metadata(&dst).expect("stat").len();
    assert_eq!(copied, size, "size mismatch on a 5 GiB file");

    // Spot-check both ends rather than hashing 5 GiB.
    let mut f = File::open(&dst).expect("open");
    let mut head = [0u8; 5];
    std::io::Read::read_exact(&mut f, &mut head).expect("read head");
    assert_eq!(&head, b"START");
    f.seek(SeekFrom::Start(size - 3)).expect("seek");
    let mut tail = [0u8; 3];
    std::io::Read::read_exact(&mut f, &mut tail).expect("read tail");
    assert_eq!(&tail, b"END");
}
