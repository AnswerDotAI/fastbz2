use std::{fs, process::Command};

use crabz2::{Level, compress};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fastbz2"))
}

#[test]
fn decode_test_index_and_list() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("sample.bz2");
    let output = directory.path().join("sample.txt");
    let index = directory.path().join("sample.fbz2i");
    let plain: Vec<_> = (0..250_000).map(|i| ((i * 31 + i / 97) & 255) as u8).collect();
    fs::write(&input, compress(&plain, Level::FASTEST)).unwrap();

    let decoded = binary().args(["decode", input.to_str().unwrap(), "-o", output.to_str().unwrap(), "-P", "2"]).status().unwrap();
    assert!(decoded.success());
    assert_eq!(fs::read(&output).unwrap(), plain);

    let tested = binary().args(["test", input.to_str().unwrap()]).status().unwrap();
    assert!(tested.success());

    let indexed = binary().args(["index", input.to_str().unwrap(), "-o", index.to_str().unwrap()]).status().unwrap();
    assert!(indexed.success());
    assert!(fs::metadata(index).unwrap().len() > 100);

    let listed = binary().args(["list", input.to_str().unwrap()]).output().unwrap();
    assert!(listed.status.success());
    assert!(String::from_utf8(listed.stdout).unwrap().contains("blocks\t"));
}

#[test]
fn corruption_has_distinct_exit_status_and_atomic_output() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("corrupt.bz2");
    let output = directory.path().join("output");
    let mut compressed = compress(b"corrupt me", Level::BEST);
    let last = compressed.len() - 2;
    compressed[last] ^= 1;
    fs::write(&input, compressed).unwrap();

    let result = binary().args(["decode", input.to_str().unwrap(), "-o", output.to_str().unwrap()]).status().unwrap();
    assert_eq!(result.code(), Some(3));
    assert!(!output.exists());
}
