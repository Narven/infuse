//! Integration tests: run the infuse binary and assert on stdout/exit code.

use std::io::Write;
use std::process::Command;

const INFUSE_BIN: &str = env!("CARGO_BIN_EXE_infuse");

#[test]
fn collect_success_returns_json_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let test_file = tmp.path().join("test_foo.py");
    std::fs::File::create(&test_file)
        .unwrap()
        .write_all(b"def test_ok(): pass")
        .unwrap();

    let out = Command::new(INFUSE_BIN)
        .args(["collect", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let manifest: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(manifest.len(), 1);
    let entry = &manifest[0];
    assert!(entry["file"].as_str().unwrap().contains("test_foo.py"));
    let tests = entry["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].as_str().unwrap(), "test_ok");
}

#[test]
fn collect_invalid_dir_exits_non_zero() {
    let out = Command::new(INFUSE_BIN)
        .args(["collect", "/nonexistent/dir/12345"])
        .output()
        .unwrap();

    assert!(!out.status.success());
}
