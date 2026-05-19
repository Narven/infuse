# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

`infuse` is a Rust CLI that speeds up **pytest's collection phase**. Pytest still imports and runs the tests — infuse only decides *which files* pytest looks at and *in what order*. It is a single-binary tool (`src/main.rs`) plus an embedded `conftest.py` template that wires pytest into it.

Beta / pre-1.0. Edition 2024, MSRV 1.85.

> **Naming history:** the project was originally called `trex` and was renamed to `infuse`. The Cargo package, binary, CLI, and env var (`INFUSE_BIN`) all use the new name. The GitHub repository URL is intentionally still `github.com/narven/trex` — leave Cargo.toml's `repository` / `homepage` / `documentation` fields alone unless asked. If you find a stray `trex` reference, it's a miss from the rename; rename it.

## Non-negotiable design goals

These two goals constrain every change. If a change can't satisfy both, push back or ask before merging it.

1. **Make pytest as fast as physically possible**, especially on large suites (thousands of files). Speed is the entire reason this exists — collection cost is what scales badly, so the project's job is to keep collection time as close to "filesystem walk + parse" as we can.

2. **Zero friction for pytest users.** The mental model for everyone — first-time pytest user, seasoned dev, team running an enormous suite — must stay: *"I run `pytest` like I always have."* In practice this means:
   - **No new flags, no new commands, no new config** to opt in beyond `infuse init` + having the binary somewhere reachable. Don't add `--infuse-something` to pytest, don't require a `pyproject.toml` section, don't introduce env vars beyond the existing `INFUSE_BIN` escape hatch.
   - **Never break a pytest run.** If infuse is missing, errors, times out, returns junk, or finds zero tests, the plugin must silently fall back to default pytest behavior. The existing try/except + manifest-None paths in the conftest are load-bearing — don't tighten them into hard failures.
   - **No surprising behavior changes.** Test selection, ordering semantics (within what pytest itself guarantees), `-k` / `-m` / parametrize / fixture behavior must look identical to vanilla pytest. If a feature would diverge from pytest's collection results, it's a regression even if it's faster.

   Anything that would force a user to learn something new, change a CI command, or debug an infuse-specific failure violates this goal.

## Commands

```bash
cargo build --release           # produces target/release/infuse
cargo test                      # runs unit tests (inline in src/main.rs) + integration tests in tests/
cargo test <name>               # single test by name, e.g. cargo test glob_to_regex_matches_test_py
cargo test --test collect_integration   # only the collect integration suite
cargo clippy -- -D warnings     # CI enforces zero warnings — match this locally before pushing
```

End-to-end check against the bundled example:

```bash
cargo build --release
cd examples/example1
./benchmark_collection.sh       # runs `uv run pytest` with and without infuse
```

## Architecture

The whole tool is ~400 LOC in `src/main.rs`. Two halves matter:

### 1. The Rust binary

Two subcommands, both defined with `clap` derive:

- **`infuse collect <root>`** — `WalkDir`s the tree, filters filenames against a glob (default `test_*.py`) converted to a regex by `glob_to_regex`, and extracts tests via two line-based regexes in `extract_tests_from_source`:
  - `^\s*class (Test\w+)\s*[(:]` at indent 0 → "current class" (accepts `:` or `(Base):`)
  - `^\s*def (test_\w+)\s*\(` → emits `test_name` (indent 0) or `ClassName::test_name` (indented)

  Output is a JSON array on stdout: `[{"file": "...", "tests": ["test_a", "TestX::test_b"]}, ...]`.

- **`infuse init`** — interactively writes a `conftest.py` (the `CONFTEST_TEMPLATE` constant) into the target dir. Reads y/N from stdin; refuses if `conftest.py` already exists.

### 2. The pytest plugin (CONFTEST_TEMPLATE)

The conftest is a **string constant inside `src/main.rs`** — there is also a tracked copy at `examples/example1/conftest.py` used for benchmarking. When you change collection behavior, **both must stay in sync** (the source of truth for users is `CONFTEST_TEMPLATE`). The `init_with_y_creates_conftest` integration test compiles the generated conftest with `python -c "compile(...)"` to catch raw-string delimiter mistakes.

How the plugin hooks pytest:

1. `pytest_configure` runs `infuse collect` once, caches the manifest plus precomputed `allowed_files` / `allowed_dirs` sets on the `config` object.
2. `pytest_ignore_collect` returns `True` for any path *not* in those sets — this is where pytest is actually saved work, because it never descends into / imports those files.
3. `pytest_collection_modifyitems` filters items down to nodeids present in infuse's manifest and reorders them to match infuse's order.

Binary resolution order in `_get_infuse_bin`: `INFUSE_BIN` env var (honored strictly — if set to a path that doesn't exist, we do NOT fall through to PATH; we fall back to default pytest) → otherwise `shutil.which("infuse")`. **If the binary is missing or fails, all hooks no-op and pytest collects normally** — never break the user's test run. `examples/example1/benchmark_collection.sh` sets `INFUSE_BIN` explicitly to this repo's `target/release/infuse`; there is no implicit "look up a path next to the conftest" lookup.

### Constraints that shape the design

- **Parser is line-and-regex, not AST.** `extract_tests_from_source` tracks two pieces of state as it walks lines:
  - `current_class`: the most recent `class Test*` at indent 0 (matched whether followed by `:` or `(Base):`). Cleared on the first indent-0 non-blank line that isn't itself a new `Test*` class — so the class body's "end" is recognised by structure, not just by seeing another `def test_*`. Nested `Test*` classes do not update it — `extract_tests_nested_class_current_behavior` pins that nested `def`s are attributed to the outer class.
  - `skip_indent`: indent of the innermost enclosing scope where a `def test_*` must be dropped — any `def` (closure parent) or any non-`Test*` class (whose methods pytest does not collect). Cleared on the first non-blank line at indent ≤ that value.

  Decision for `def test_*`: indent 0 → top-level test (and class context is now stale anyway); else if `skip_indent` is set → drop (closure or non-Test class method); else if `current_class` is set → `Class::name`; else → top-level test (covers `if:` / `try:` / `with:` blocks at module level).

  The order of those branches matters: `skip_indent` is checked *before* `current_class` so that nested non-Test classes and closures inside test methods win over an outer `Test*` class.

  Limitations to be aware of when touching this:
  - Multi-line class headers like `class TestFoo(\n    Base,\n):` set `current_class` on the opening line, but the closing `):` is an indent-0 statement that clears it again. Test methods after such a header will be missed.
  - `async def test_*` is not matched.
  - `def test_*` inside a string literal or docstring will be falsely matched.
- **NodeId match is exact.** The plugin filters by `f"{file}::{test_id}"` against `item.nodeid`. Parametrized IDs like `test_foo[case1]` are not in infuse's manifest and will be filtered out. Be careful when changing the filter — `modifyitems` is what hides tests, `ignore_collect` is what saves time.
- **`pytest_ignore_collect` is where the speedup lives.** `modifyitems` runs *after* pytest has already imported every test module, so filtering there doesn't help collection time. The comment in `examples/example1/benchmark_collection.sh` explains why infuse can even appear slower on tiny suites (subprocess + filter overhead with no I/O savings).

## Testing layout

- Unit tests live inline in `src/main.rs` under `#[cfg(test)] mod tests` — pure functions (`glob_to_regex`, `extract_tests_from_source`, `collect_tests`).
- Integration tests in `tests/` invoke the built binary via `env!("CARGO_BIN_EXE_infuse")`:
  - `collect_integration.rs` — runs `infuse collect` against a tempdir, asserts JSON shape.
  - `init_integration.rs` — pipes `y\n` / `n\n` to `infuse init` stdin, asserts conftest creation, and `compile()`s the generated file with `python3` to catch malformed templates.

## CI

`.github/workflows/ci.yml` runs `cargo test`, `cargo build --release`, then `cargo clippy -- -D warnings` on Ubuntu. Clippy warnings fail the build. `.github/workflows/release.yml` builds release binaries on Linux/macOS/Windows and uploads them as `infuse-<platform>` artifacts.
