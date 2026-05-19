//! Integration tests for `infuse init`: stdin n/y and conftest creation.

use std::io::Write;
use std::process::{Command, Stdio};

const INFUSE_BIN: &str = env!("CARGO_BIN_EXE_infuse");

#[test]
fn init_with_n_does_not_create_conftest() {
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(INFUSE_BIN)
        .arg("init")
        .arg(tmp.path().to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"n\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());

    let conftest = tmp.path().join("conftest.py");
    assert!(
        !conftest.exists(),
        "conftest.py should not be created when user says n"
    );
}

#[test]
fn init_with_y_creates_conftest() {
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(INFUSE_BIN)
        .arg("init")
        .arg(tmp.path().to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"y\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let conftest = tmp.path().join("conftest.py");
    assert!(
        conftest.exists(),
        "conftest.py should be created when user says y"
    );
    let content = std::fs::read_to_string(&conftest).unwrap();
    assert!(
        content.contains("pytest_configure"),
        "conftest should contain pytest_configure"
    );

    // The generated file must be valid Python — `python -c "compile(...)"`
    // catches malformed templates (e.g. raw-string delimiter mistakes).
    let py = Command::new("python3")
        .args([
            "-c",
            "import sys; compile(open(sys.argv[1]).read(), sys.argv[1], 'exec')",
            conftest.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        py.status.success(),
        "generated conftest.py failed to compile as Python: {}",
        String::from_utf8_lossy(&py.stderr)
    );
}
