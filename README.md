# nmpool

A local, private dependency-install cache for macOS and Windows.

Cache entries are platform-specific. Existing installs are never adopted, linked,
replaced or deleted. Every restored worktree gets its own writable files.

## Build

Requires Rust 1.89+ and Git. Prepare/restore also require native Node and npm.

```sh
cargo build --release --locked
python scripts/check.py
```

Native CI runs on macOS and Windows. Windows uses `nmpool.exe`; no WSL or admin
rights are required for the baseline. Both platforms build their own cache entries.

## Development checks

`python scripts/check.py` runs the same formatting, strict Clippy, locked tests
and warning-free documentation checks used by native CI. Use Python 3 (`python3`
on Macs without a `python` alias); Make is an optional convenience. Clippy enables
all, pedantic, nursery and cargo groups plus explicit panic, unwrap, indexing and
debug-output restrictions. Complexity limits match Dossier/Rooms (20 cognitive,
100 lines, six arguments). Scoped exceptions carry reasons.

Every PR also checks the declared Rust 1.89 minimum and runs `cargo audit --deny
warnings` with no advisory ignores. Install `cargo-audit` to run `make audit`
locally. Optional manual workflows produce native macOS/Windows LCOV coverage
and macOS mutation reports; they are audits, not claimed coverage or mutation
score gates. Their local equivalents require `cargo-llvm-cov` plus LLVM tools, or
`cargo-mutants`. Workflow success does not imply branch protection is configured.

## Use

```sh
nmpool census --repo /path/to/repo --json
nmpool prepare --package /path/to/package --cache /path/to/private-cache
nmpool restore --package /path/to/new-worktree/package --cache /path/to/private-cache
nmpool inspect --cache /path/to/private-cache --key KEY_FROM_PREPARE
```

Use native Windows paths for the same arguments in PowerShell. Node is discovered
on PATH; `--node` selects an executable. npm's standard layout is detected, or supply
`--npm-cli C:\path\to\node_modules\npm\bin\npm-cli.js` explicitly. npm runs through
Node, avoiding shell parsing of an npm.cmd command line.

`census` is read-only. Repeat `--repo` to scan more than one repository; registered
worktrees are deduplicated. Default package depth is two; `--max-depth` permits up to
eight. JSON includes incomplete-scan errors, candidate input groups and link states.
Exit 0 means complete within scope, 2 partial census, 1 failure.

`prepare` creates a fresh install in private staging, never in the source package.
`restore` only accepts an absent node_modules. It leaves a `.nmpool.lock` in the
destination package to coordinate callers; add it to your local ignore rules if
needed. Do not run other installers/watchers in that destination during restore.
`inspect` verifies the full cache entry without creating files.

## Deliberately narrow first release

- npm package-lock v3 and integrity-pinned public npm registry dependencies.
- No lifecycle scripts, workspaces, local/Git dependencies or arbitrary npmrc.
- All installs use `npm ci --ignore-scripts` with controlled configuration.
- Full artifact verification on every restore; no trust based on lock equality alone.
- Native private copying, with clone use and physical storage savings **unmeasured**.
- No shared writable node_modules, adoption, GC, remote cache or plugin framework.

Roxiq's install scripts are currently refused; Ivy MCP is the initial supported
personal workload. This is a usable experimental mechanism, not a performance claim.
Failed staging and preparation logs are retained under `CACHE/staging` for inspection;
they can consume disk space. There is no automatic cleanup. Never point the cache at
an existing dependency tree or an unrelated nonempty directory.

See [design and threat boundary](docs/design.md) and [validation](docs/validation.md).
