# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - unreleased

### Added

- Read-only `assess` reports sharing requirements together without executing npm.
- `protection-probe` tests consumer write denial in a disposable native fixture;
  live sharing and adoption remain unimplemented.

- Copy-mode private install cache: `prepare`, `restore`, `status`, `inspect`,
  `explain`, `scan` (alias `census`).
- Native macOS and Windows support; installs are never adopted, linked,
  replaced or deleted.
- Clean profile only: npm lockfile v3, public registry, scripts disabled.

### Fixed

- Normalize JSON CRLF/LF input identity across Git worktrees; explain formatting
  differences and include the requested key and cache path in cache-miss errors.
- Input schema v2 requires preparing new entries; old caches and installs are
  preserved, not migrated or deleted.
