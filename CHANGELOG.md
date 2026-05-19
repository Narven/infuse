# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial release.
- `infuse collect <root_dir>` subcommand: recursive file discovery with `--pattern` (default `test_*.py`), regex-based extraction of `test_*` functions and `Test*` class methods, JSON manifest on stdout.
- Pytest plugin (conftest): `pytest_configure` runs infuse once and caches manifest + allowed files/dirs; `pytest_ignore_collect` restricts collection to infuse-listed paths; `pytest_collection_modifyitems` filters and reorders items using cached manifest. Fallback to default pytest behavior when infuse binary is missing or fails.

### Changed

- None.

### Deprecated

- None.

### Removed

- None.

### Fixed

- None.

### Security

- None.
