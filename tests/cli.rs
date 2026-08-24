use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, UNIX_EPOCH},
};

use crabz2::{Level, compress};
use flate2::{Compression, write::GzEncoder};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fastbz2"))
}

fn write_compressed(path: &Path, plain: &[u8]) {
    fs::write(path, compress(plain, Level::FASTEST)).unwrap();
}

fn write_gzip(path: &Path, plain: &[u8]) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(plain).unwrap();
    fs::write(path, encoder.finish().unwrap()).unwrap();
}

#[test]
fn decode_test_index_and_list() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("sample.bz2");
    let output = directory.path().join("sample");
    let index = directory.path().join("sample.fbz2i");
    let plain: Vec<_> = (0..250_000).map(|i| ((i * 31 + i / 97) & 255) as u8).collect();
    write_compressed(&input, &plain);

    let decoded = binary().args([input.to_str().unwrap(), "-P", "2"]).status().unwrap();
    assert!(decoded.success());
    assert_eq!(fs::read(&output).unwrap(), plain);

    let stdout = binary().args([input.to_str().unwrap(), "-o", "-"]).output().unwrap();
    assert!(stdout.status.success());
    assert_eq!(stdout.stdout, plain);

    let tested = binary().args(["--test", input.to_str().unwrap()]).status().unwrap();
    assert!(tested.success());

    let indexed = binary().args(["--index", input.to_str().unwrap(), "-o", index.to_str().unwrap()]).status().unwrap();
    assert!(indexed.success());
    assert!(fs::metadata(index).unwrap().len() > 100);

    let listed = binary().args(["--list", input.to_str().unwrap()]).output().unwrap();
    assert!(listed.status.success());
    assert!(String::from_utf8(listed.stdout).unwrap().contains("blocks\t"));

    let conflicting = binary().args(["--test", "--list", input.to_str().unwrap()]).output().unwrap();
    assert_eq!(conflicting.status.code(), Some(2));
}

#[test]
fn stdin_decodes_to_stdout() {
    let plain = b"stdin uses the same decode options";
    let compressed = compress(plain, Level::BEST);
    let mut child = binary().args(["-", "-P", "2", "--memory-limit", "128M"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&compressed).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, plain);
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

    let result = binary().args([input.to_str().unwrap(), "-o", output.to_str().unwrap()]).status().unwrap();
    assert_eq!(result.code(), Some(3));
    assert!(!output.exists());
}

#[test]
fn multiple_inputs_support_output_directory_and_skip_existing() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.bz2");
    let second = directory.path().join("second.bz2");
    let output_dir = directory.path().join("decoded");
    write_compressed(&first, b"first contents");
    write_compressed(&second, b"second contents");

    let decoded = binary().args(["-C", output_dir.to_str().unwrap(), first.to_str().unwrap(), second.to_str().unwrap()]).output().unwrap();
    assert!(decoded.status.success(), "{}", String::from_utf8_lossy(&decoded.stderr));
    assert_eq!(fs::read(output_dir.join("first")).unwrap(), b"first contents");
    assert_eq!(fs::read(output_dir.join("second")).unwrap(), b"second contents");

    fs::write(output_dir.join("first"), b"keep me").unwrap();
    let skipped = binary().args(["--skip-existing", "-C", output_dir.to_str().unwrap(), first.to_str().unwrap(), second.to_str().unwrap()]).output().unwrap();
    assert!(skipped.status.success());
    assert_eq!(fs::read(output_dir.join("first")).unwrap(), b"keep me");
    assert!(String::from_utf8(skipped.stderr).unwrap().contains("skipping existing"));

    let quiet =
        binary().args(["--quiet", "--skip-existing", "-C", output_dir.to_str().unwrap(), first.to_str().unwrap(), second.to_str().unwrap()]).output().unwrap();
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());

    let replaced = binary().args(["--force", "-C", output_dir.to_str().unwrap(), first.to_str().unwrap(), second.to_str().unwrap()]).output().unwrap();
    assert!(replaced.status.success());
    assert_eq!(fs::read(output_dir.join("first")).unwrap(), b"first contents");

    let rejected = binary().args([first.to_str().unwrap(), second.to_str().unwrap(), "-o", output_dir.join("one").to_str().unwrap()]).output().unwrap();
    assert_eq!(rejected.status.code(), Some(2));
}

#[test]
fn remove_input_happens_only_after_success() {
    let directory = tempfile::tempdir().unwrap();
    let valid = directory.path().join("valid.bz2");
    let corrupt = directory.path().join("corrupt.bz2");
    write_compressed(&valid, b"remove after success");
    fs::write(&corrupt, b"not bzip2").unwrap();

    let decoded = binary().args(["--rm", valid.to_str().unwrap()]).output().unwrap();
    assert!(decoded.status.success());
    assert!(!valid.exists());
    assert_eq!(fs::read(directory.path().join("valid")).unwrap(), b"remove after success");

    let failed = binary().args(["--rm", corrupt.to_str().unwrap()]).output().unwrap();
    assert!(!failed.status.success());
    assert!(corrupt.exists());
}

#[test]
fn decode_preserves_modified_time_and_permissions() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("metadata.bz2");
    let output = directory.path().join("metadata");
    write_compressed(&input, b"metadata");
    let modified = UNIX_EPOCH + Duration::from_secs(1_700_000_123);
    fs::OpenOptions::new().write(true).open(&input).unwrap().set_times(fs::FileTimes::new().set_modified(modified)).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&input, fs::Permissions::from_mode(0o640)).unwrap();
    }

    let decoded = binary().arg(input.to_str().unwrap()).output().unwrap();
    assert!(decoded.status.success());
    let input_metadata = fs::metadata(input).unwrap();
    let output_metadata = fs::metadata(output).unwrap();
    assert_eq!(output_metadata.modified().unwrap(), input_metadata.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(output_metadata.permissions().mode() & 0o777, 0o640);
    }
}

#[test]
fn max_output_is_enforced_before_persisting() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("limited.bz2");
    let output = directory.path().join("limited");
    let plain = vec![b'x'; 20_000];
    write_compressed(&input, &plain);

    let rejected = binary().args(["--max-output", "19999", input.to_str().unwrap()]).output().unwrap();
    assert_eq!(rejected.status.code(), Some(3));
    assert!(!output.exists());
    assert!(String::from_utf8(rejected.stderr).unwrap().contains("decoded output exceeds"));

    let accepted = binary().args(["--max-output", "20K", input.to_str().unwrap()]).output().unwrap();
    assert!(accepted.status.success());
    assert_eq!(fs::read(output).unwrap(), plain);
}

#[test]
fn list_json_describes_one_or_many_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.bz2");
    let second = directory.path().join("second.bz2");
    write_compressed(&first, b"first");
    write_compressed(&second, b"second");

    let single = binary().args(["--list", "--json", first.to_str().unwrap()]).output().unwrap();
    assert!(single.status.success());
    let value: serde_json::Value = serde_json::from_slice(&single.stdout).unwrap();
    assert_eq!(value["input"], first.to_str().unwrap());
    assert_eq!(value["decoded_bytes"], 5);
    assert_eq!(value["streams"].as_array().unwrap().len(), 1);
    assert_eq!(value["blocks"].as_array().unwrap().len(), 1);

    let multiple = binary().args(["--list", "--json", first.to_str().unwrap(), second.to_str().unwrap()]).output().unwrap();
    assert!(multiple.status.success());
    let value: serde_json::Value = serde_json::from_slice(&multiple.stdout).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 2);

    let limited = binary().args(["--list", "--max-output", "4", first.to_str().unwrap()]).output().unwrap();
    assert_eq!(limited.status.code(), Some(3));

    let missing_mode = binary().args(["--json", first.to_str().unwrap()]).output().unwrap();
    assert_eq!(missing_mode.status.code(), Some(2));
}

#[test]
fn gzip_extension_selects_decoder_across_cli_modes() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("sample.gz");
    let output = directory.path().join("sample");
    let plain: Vec<_> = (0..250_000).map(|i| ((i * 31 + i / 97) & 255) as u8).collect();
    write_gzip(&input, &plain);

    let decoded = binary().arg(input.to_str().unwrap()).output().unwrap();
    assert!(decoded.status.success(), "{}", String::from_utf8_lossy(&decoded.stderr));
    assert_eq!(fs::read(output).unwrap(), plain);

    let tested = binary().args(["--test", input.to_str().unwrap()]).output().unwrap();
    assert!(tested.status.success());

    let listed = binary().args(["--list", "--json", input.to_str().unwrap()]).output().unwrap();
    assert!(listed.status.success());
    let value: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(value["format"], "gzip");
    assert_eq!(value["decoded_bytes"], plain.len());
    assert_eq!(value["members"].as_array().unwrap().len(), 1);
    assert!(!value["blocks"].as_array().unwrap().is_empty());

    let indexed = binary().args(["--index", input.to_str().unwrap()]).output().unwrap();
    assert_eq!(indexed.status.code(), Some(2));
    assert!(String::from_utf8(indexed.stderr).unwrap().contains("only for bzip2"));
}

#[test]
fn gzip_magic_fallback_stdin_limits_and_corruption_work() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("mystery.data");
    let output = directory.path().join("mystery.data.out");
    let plain = b"gzip magic fallback".repeat(2_000);
    write_gzip(&input, &plain);

    let decoded = binary().arg(input.to_str().unwrap()).output().unwrap();
    assert!(decoded.status.success());
    assert_eq!(fs::read(&output).unwrap(), plain);
    fs::remove_file(&output).unwrap();

    let compressed = fs::read(&input).unwrap();
    let mut child = binary().arg("-").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&compressed).unwrap();
    let stdin = child.wait_with_output().unwrap();
    assert!(stdin.status.success());
    assert_eq!(stdin.stdout, plain);

    let limited = binary().args(["--max-output", "1K", input.to_str().unwrap()]).output().unwrap();
    assert_eq!(limited.status.code(), Some(3));
    assert!(!output.exists());

    let mut corrupt = compressed;
    let crc = corrupt.len() - 8;
    corrupt[crc] ^= 1;
    fs::write(&input, corrupt).unwrap();
    let rejected = binary().arg(input.to_str().unwrap()).output().unwrap();
    assert_eq!(rejected.status.code(), Some(3));
    assert!(!output.exists());
}

#[test]
fn mixed_bzip2_and_gzip_inputs_share_output_policy() {
    let directory = tempfile::tempdir().unwrap();
    let bzip2 = directory.path().join("first.bz2");
    let gzip = directory.path().join("second.gz");
    let tgz = directory.path().join("bundle.tgz");
    let output_dir = directory.path().join("decoded");
    write_compressed(&bzip2, b"bzip2");
    write_gzip(&gzip, b"gzip");
    write_gzip(&tgz, b"tar payload");

    let decoded = binary().args(["-C", output_dir.to_str().unwrap(), bzip2.to_str().unwrap(), gzip.to_str().unwrap(), tgz.to_str().unwrap()]).output().unwrap();
    assert!(decoded.status.success(), "{}", String::from_utf8_lossy(&decoded.stderr));
    assert_eq!(fs::read(output_dir.join("first")).unwrap(), b"bzip2");
    assert_eq!(fs::read(output_dir.join("second")).unwrap(), b"gzip");
    assert_eq!(fs::read(output_dir.join("bundle.tar")).unwrap(), b"tar payload");
}
