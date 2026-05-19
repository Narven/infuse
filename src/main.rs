use clap::{Parser, Subcommand};
use regex::Regex;
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use walkdir::WalkDir;

const CONFTEST_TEMPLATE: &str = r#"""Pytest plugin: override collection using Rust (trex) for discovery."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path


def _get_trex_bin():
    # TREX_BIN wins if set (used as an escape hatch / for dev builds); otherwise
    # look up `trex` on PATH. Returns None when neither resolves to a real file.
    env_bin = os.environ.get("TREX_BIN")
    if env_bin:
        return env_bin
    return shutil.which("trex")


def _run_trex_collect(rootdir: Path, trex_bin: str) -> list | None:
    try:
        result = subprocess.run(
            [trex_bin, "collect", str(rootdir)],
            capture_output=True,
            text=True,
            timeout=30,
            cwd=str(rootdir),
        )
        result.check_returncode()
        return json.loads(result.stdout)
    except (subprocess.CalledProcessError, json.JSONDecodeError, FileNotFoundError):
        return None


def _allowed_sets_from_manifest(manifest: list) -> tuple[set[str], set[str]]:
    allowed_files = set()
    allowed_dirs = set()
    for entry in manifest:
        f = entry["file"].replace("\\", "/")
        allowed_files.add(f)
        parts = f.split("/")
        for i in range(len(parts)):
            prefix = "/".join(parts[:i]) if i else "."
            allowed_dirs.add(prefix)
    return allowed_files, allowed_dirs


def pytest_configure(config):
    rootdir = config.rootpath
    if not rootdir:
        rootdir = Path.cwd()
    else:
        rootdir = Path(rootdir)
    trex_bin = _get_trex_bin()
    if not trex_bin or not Path(trex_bin).exists():
        return
    manifest = _run_trex_collect(rootdir, trex_bin)
    if manifest is None:
        return
    config._trex_manifest = manifest
    config._trex_allowed_files, config._trex_allowed_dirs = _allowed_sets_from_manifest(
        manifest
    )


def pytest_ignore_collect(collection_path, config):
    manifest = getattr(config, "_trex_manifest", None)
    if manifest is None:
        return False
    allowed_files = getattr(config, "_trex_allowed_files", set())
    allowed_dirs = getattr(config, "_trex_allowed_dirs", set())
    rootdir = Path(config.rootpath).resolve()
    try:
        rel = collection_path.resolve().relative_to(rootdir)
    except ValueError:
        return False
    key = str(rel).replace("\\", "/") or "."
    if collection_path.is_file():
        return key not in allowed_files
    if collection_path.is_dir():
        return key not in allowed_dirs
    return False


def pytest_collection_modifyitems(session, config, items):
    manifest = getattr(config, "_trex_manifest", None)
    if manifest is None:
        trex_bin = _get_trex_bin()
        rootdir = config.rootpath
        if not rootdir:
            rootdir = Path.cwd()
        else:
            rootdir = Path(rootdir)
        if not trex_bin or not Path(trex_bin).exists():
            return
        manifest = _run_trex_collect(rootdir, trex_bin)
        if manifest is None:
            return
        config._trex_manifest = manifest
        config._trex_allowed_files, config._trex_allowed_dirs = _allowed_sets_from_manifest(
            manifest
        )

    rust_order = []
    for entry in manifest:
        file_path = entry["file"]
        for test_id in entry["tests"]:
            rust_order.append(f"{file_path}::{test_id}")

    rust_set = set(rust_order)
    items[:] = [item for item in items if item.nodeid in rust_set]
    order_map = {nodeid: i for i, nodeid in enumerate(rust_order)}
    items.sort(key=lambda item: order_map.get(item.nodeid, float("inf")))
"#;

#[derive(Parser)]
#[command(name = "trex")]
#[command(about = "Rust-powered pytest collection")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Discover and list Python tests (for pytest_collection_modifyitems)
    Collect {
        /// Root directory to search for test files
        root_dir: std::path::PathBuf,

        /// Glob pattern for test files (default: test_*.py)
        #[arg(long, default_value = "test_*.py")]
        pattern: String,
    },

    /// Create conftest.py in the project dir if missing (prompts before writing)
    Init {
        /// Directory to add conftest.py (default: current directory)
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },
}

#[derive(Debug, Serialize)]
struct FileTests {
    file: String,
    tests: Vec<String>,
}

fn glob_to_regex(glob: &str) -> regex::Regex {
    let mut re = String::from("^");
    for c in glob.chars() {
        match c {
            '.' => re.push_str("\\."),
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            _ if c.is_ascii_alphanumeric() || c == '_' || c == '-' => re.push(c),
            _ => {
                re.push('\\');
                re.push(c);
            }
        }
    }
    re.push('$');
    Regex::new(&re).expect("invalid glob pattern")
}

fn extract_tests_from_source(source: &str) -> Vec<String> {
    let class_re = Regex::new(r"^\s*class (Test\w+)\s*[(:]").unwrap();
    let any_class_re = Regex::new(r"^(\s*)class \w+").unwrap();
    let test_def_re = Regex::new(r"^\s*def (test_\w+)\s*\(").unwrap();
    let any_def_re = Regex::new(r"^(\s*)def \w+\s*\(").unwrap();

    let mut tests = Vec::new();
    let mut current_class: Option<String> = None;
    // Indent of the innermost enclosing scope where `def test_*` should be
    // suppressed: any `def` (closure parent) or any non-`Test*` class (whose
    // methods pytest does not collect).
    let mut skip_indent: Option<usize> = None;

    for line in source.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        if let Some(s_indent) = skip_indent {
            if indent <= s_indent {
                skip_indent = None;
            }
        }

        // Any indent-0 statement that isn't itself starting a Test* class
        // ends the previous top-level class's body, so its name is stale.
        if indent == 0 {
            current_class = None;
        }

        if let Some(cap) = class_re.captures(line) {
            if indent == 0 {
                current_class = Some(cap[1].to_string());
            }
            continue;
        }

        if let Some(cap) = test_def_re.captures(line) {
            let test_name = cap[1].to_string();
            if indent == 0 {
                tests.push(test_name);
            } else if skip_indent.is_none() {
                if let Some(ref class_name) = current_class {
                    tests.push(format!("{}::{}", class_name, test_name));
                } else {
                    tests.push(test_name);
                }
            }
            skip_indent = Some(indent);
            continue;
        }

        if let Some(cap) = any_def_re.captures(line) {
            skip_indent = Some(cap[1].len());
            continue;
        }

        if let Some(cap) = any_class_re.captures(line) {
            skip_indent = Some(cap[1].len());
        }
    }

    tests
}

fn collect_tests(root_dir: &Path, pattern: &str) -> Vec<FileTests> {
    let pattern_re = glob_to_regex(pattern);
    let mut results = Vec::new();

    for entry in WalkDir::new(root_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !pattern_re.is_match(file_name) {
            continue;
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let tests = extract_tests_from_source(&content);
        if tests.is_empty() {
            continue;
        }

        let file_path = path
            .strip_prefix(root_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        results.push(FileTests {
            file: file_path,
            tests,
        });
    }

    results
}

fn run_collect(root_dir: &Path, pattern: &str) {
    let results = collect_tests(root_dir, pattern);
    let json = serde_json::to_string(&results).expect("serialize");
    println!("{}", json);
}

fn run_init(dir: &Path) {
    if !dir.is_dir() {
        eprintln!("trex init: not a directory: {}", dir.display());
        std::process::exit(1);
    }
    let conftest_path = dir.join("conftest.py");
    if conftest_path.exists() {
        eprintln!("conftest.py already exists in {}", dir.display());
        return;
    }
    eprint!("No conftest.py detected. Generate one? [y/N] ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        eprintln!("trex init: could not read input");
        std::process::exit(1);
    }
    let answer = line.trim().to_lowercase();
    if answer != "y" && answer != "yes" {
        eprintln!("Skipped.");
        return;
    }
    if fs::write(&conftest_path, CONFTEST_TEMPLATE).is_err() {
        eprintln!("trex init: failed to write {}", conftest_path.display());
        std::process::exit(1);
    }
    eprintln!("Wrote {}", conftest_path.display());
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Collect { root_dir, pattern } => {
            if !root_dir.is_dir() {
                eprintln!("trex: root_dir is not a directory: {}", root_dir.display());
                std::process::exit(1);
            }
            run_collect(root_dir, pattern);
        }
        Commands::Init { dir } => run_init(dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn glob_to_regex_matches_test_py() {
        let re = glob_to_regex("test_*.py");
        assert!(re.is_match("test_foo.py"));
        assert!(re.is_match("test_operations.py"));
        assert!(!re.is_match("foo.py"));
        assert!(!re.is_match("test_foo.txt"));
    }

    #[test]
    fn glob_to_regex_star_and_dot_escaped() {
        let re = glob_to_regex("*.py");
        assert!(re.is_match("a.py"));
        assert!(re.is_match("test_foo.py"));
        assert!(!re.is_match("axpy"));
    }

    #[test]
    fn extract_tests_top_level_function() {
        let source = "def test_foo(): pass";
        assert_eq!(extract_tests_from_source(source), vec!["test_foo"]);
    }

    #[test]
    fn extract_tests_class_method() {
        let source = r#"
class TestBar:
    def test_baz(self):
        pass
"#;
        assert_eq!(extract_tests_from_source(source), vec!["TestBar::test_baz"]);
    }

    #[test]
    fn extract_tests_mixed_top_level_and_class() {
        let source = r#"
def test_standalone():
    pass

class TestFoo:
    def test_method(self):
        pass
"#;
        let got = extract_tests_from_source(source);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], "test_standalone");
        assert_eq!(got[1], "TestFoo::test_method");
    }

    #[test]
    fn extract_tests_empty() {
        assert!(extract_tests_from_source("").is_empty());
        assert!(extract_tests_from_source("def foo(): pass").is_empty());
        assert!(extract_tests_from_source("class Bar: pass").is_empty());
    }

    #[test]
    fn extract_tests_indented_under_block_collected_as_top_level() {
        // pytest collects `test_foo` here because it's bound at module level after the if runs.
        let source = r#"
if True:
    def test_foo():
        pass
"#;
        assert_eq!(extract_tests_from_source(source), vec!["test_foo"]);
    }

    #[test]
    fn extract_tests_closure_inside_function_is_dropped() {
        // pytest does NOT collect closures defined inside another function.
        let source = r#"
def helper():
    def test_inner():
        pass
"#;
        assert!(extract_tests_from_source(source).is_empty());
    }

    #[test]
    fn extract_tests_skips_methods_of_non_test_class() {
        // A non-Test class following a Test class must NOT inherit the Test*
        // class context — its `def test_*` methods are not pytest tests.
        let source = r#"
class TestFoo:
    def test_a(self):
        pass

class _Helper:
    def test_x(self):
        pass
"#;
        assert_eq!(extract_tests_from_source(source), vec!["TestFoo::test_a"]);
    }

    #[test]
    fn extract_tests_top_level_def_ends_class_context() {
        // A top-level non-test `def` between a Test class and a later
        // module-level `def test_*` must not leak the prior class name.
        let source = r#"
class TestFoo:
    pass

def some_helper():
    pass

if True:
    def test_x():
        pass
"#;
        assert_eq!(extract_tests_from_source(source), vec!["test_x"]);
    }

    #[test]
    fn extract_tests_class_with_base() {
        // Test classes can declare bases — class_re must accept `(` after the
        // name, not only `:`.
        let source = r#"
class TestFoo(unittest.TestCase):
    def test_a(self):
        pass
"#;
        assert_eq!(extract_tests_from_source(source), vec!["TestFoo::test_a"]);
    }

    #[test]
    fn extract_tests_closure_inside_test_method_is_dropped() {
        // A nested `def test_inner` inside a real test method is a closure,
        // not a method of the Test class — pytest does not collect it.
        let source = r#"
class TestFoo:
    def test_a(self):
        def test_inner():
            pass
"#;
        assert_eq!(extract_tests_from_source(source), vec!["TestFoo::test_a"]);
    }

    #[test]
    fn extract_tests_nested_class_current_behavior() {
        // Current code only tracks one class (indent 0); inner class methods use outer class name or are skipped
        let source = r#"
class TestOuter:
    class TestInner:
        def test_inner(self):
            pass
"#;
        let got = extract_tests_from_source(source);
        // TestInner is at indent 4, so current_class stays TestOuter; test_inner is indented > 0 so we get TestOuter::test_inner
        assert_eq!(got, vec!["TestOuter::test_inner"]);
    }

    #[test]
    fn collect_tests_finds_file_and_extracts_tests() {
        let tmp = tempfile::tempdir().unwrap();
        let test_file = tmp.path().join("test_sample.py");
        let content = r#"
def test_ok():
    pass

class TestFoo:
    def test_bar(self):
        pass
"#;
        std::fs::File::create(&test_file)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        let results = collect_tests(tmp.path(), "test_*.py");
        assert_eq!(results.len(), 1);
        assert!(results[0].file.contains("test_sample.py"));
        assert_eq!(results[0].tests.len(), 2);
        assert_eq!(results[0].tests[0], "test_ok");
        assert_eq!(results[0].tests[1], "TestFoo::test_bar");
    }
}
