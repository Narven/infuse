#!/usr/bin/env bash
# Compare collection and full run: with trex (Rust) vs default pytest.
# Do not use set -e so both runs always execute even if one fails.
#
# Why "with trex" can be slower for small suites:
# - Pytest always runs its full collection first (discover files, import modules, build Items).
# - Only then does pytest_collection_modifyitems run and call the trex subprocess.
# - So we pay: Python collection + subprocess spawn + trex + filter. Rust never replaces
#   Python's work; it runs in addition. For 59 tests in one file, subprocess overhead
#   dominates. Trex would only win on very large suites (e.g. 1000+ files) or if we
#   hooked pytest_ignore_collect so Python skips files not in trex's manifest.
cd "$(dirname "$0")"

# Point the conftest at this repo's dev build of trex. The conftest itself no
# longer looks up `../../target/release/trex` relative to its own location — it
# only honors TREX_BIN or `trex` on PATH — so we set it explicitly here.
export TREX_BIN="$(cd ../.. && pwd)/target/release/trex"

# Single-line timing so we don't rely on grep (which can break the script)
export TIMEFORMAT='real %R s'

echo "=== Collection only (no test run) ==="
echo -n "With trex:    "
time (uv run pytest --collect-only -q 2>/dev/null) 2>&1
echo -n "Without trex: "
time (TREX_BIN=/nonexistent uv run pytest --collect-only -q 2>/dev/null) 2>&1

echo ""
echo "=== Full run ==="
echo -n "With trex:    "
time (uv run pytest -q 2>/dev/null) 2>&1
echo -n "Without trex: "
time (TREX_BIN=/nonexistent uv run pytest -q 2>/dev/null) 2>&1
