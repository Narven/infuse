"""Pytest plugin: override collection using Rust (trex) for discovery."""

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
