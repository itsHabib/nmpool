# nmpool

A local, private dependency-install cache for macOS and Windows.

[![CI](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Cache entries are platform-specific. Existing installs are never adopted, linked,
replaced or deleted. Every restored worktree gets its own writable files.

## Start here

**Experimental.** Native macOS/Windows checks run on every PR. Real Windows-workload
acceptance remains open. The small Ivy trial restored in 1.189 s versus 0.425 s for warm npm ci:
performance benefit is not established.

On a Windows machine, follow the [Windows installation and trial guide](docs/windows-trial.md).
It covers installation, the binary alternative, compatibility checks, disposable
worktrees, correctness checks, timing, troubleshooting and cleanup.
Why copy mode is not the end of the story, and what comes next, is in
[docs/direction.md](docs/direction.md).

| Command | What it does | Changes |
|---|---|---|
| `scan` (alias of `census`) | Inventory Git worktrees and identify supported input candidates | None; no npm execution |
| `prepare` | Build a cache entry with fresh npm ci, or verify an existing matching entry | Cache only; not the source install |
| `restore` | Verify and copy an entry into an absent node_modules | New private install plus a destination lock |
| `inspect` | Read the receipt and verify every cached artifact | None |
| `status` | Compare this restored install with its receipt and current inputs/runtime | No project/cache writes |
| `explain` | Compare two packages' requested installs and name differing inputs | No project/cache writes |

`prepare` does not capture an existing install. `restore` does not fetch packages
or fall back to npm on a miss. `inspect` verifies the cache, not application tests.
There is no automatic integration with Git worktree creation.

## Daily use

```sh
# Inventory first; unsupported_reason explains packages outside this profile.
nmpool scan --repo /path/to/repo --json
# See whether two worktrees need the same installation.
nmpool explain --package /path/to/first/package --against /path/to/second/package
# Prepare once, then restore only into a worktree without node_modules.
nmpool prepare --package /path/to/first/package --cache /path/to/private-cache
nmpool restore --package /path/to/second/package --cache /path/to/private-cache
# Check before running the package's own tests/build.
nmpool status --package /path/to/second/package
```

`status` prints JSON. Exit 0 means inputs/runtime and installed files match the
restoration snapshot; exit 2 means `drifted`, `untracked`, or `absent`; exit 1 means
verification failed. Drift lists added/removed/modified files separately from
input differences. Unsupported current inputs appear in `input_error`. No receipt
means untracked, including installs from older nmpool builds; no adoption occurs.

`explain` names differing input fields (for example `/inputs/files/package-lock.json`)
and emits both keys. It compares raw file fingerprints, not individual dependency
version changes. Branch names are context: different branches can request the same
install. By default it compares both packages under the selected current Node/npm;
use `--against-node` and `--against-npm-cli` to select a different second runtime.
A successful comparison exits 0 even when requirements differ.

Both commands accept `--node` and `--npm-cli`. They execute local Node/npm identity
probes and Git reads, but no install commands; probes create temporary files outside
the project. Stop concurrent installers/editors for a consistent snapshot.

Every new restore includes `node_modules/.nmpool-restore.json`: the original input
and artifact receipt, restoration time, and available Git branch/commit context.
The record is published atomically with the install, works without the cache, and
moves with the worktree. It is local provenance, not a tamper-proof audit or a record
of which process/agent changed files. Do not edit the receipt. A reserved-name
collision is refused. Running npm ci may remove it, making the install untracked.

`status` never repairs, deletes, or refreshes an install. If inputs changed, prepare
the new requirement and use a fresh worktree. A clean status does not replace your
application tests or establish continuous monitoring.

## Build

Requires Rust 1.89+ and Git. Prepare/restore also require native Node and npm.

```sh
git clone https://github.com/itsHabib/nmpool.git
cd nmpool
cargo install --path . --locked
nmpool --version
python3 scripts/smoke.py --binary nmpool
```

The smoke test needs Python 3 and native Node/npm. It creates a disposable empty
package and retains its evidence at the printed path. On Windows, use `python`
in place of `python3`. See the [release guide](docs/releasing.md) for release scope
and checks. This experimental CLI is installed from source; it is not published
on crates.io.

Native CI runs on macOS and Windows. Windows uses `nmpool.exe`; no WSL or admin
rights are required for the baseline. Both platforms build their own cache entries.

## Development checks

`python scripts/check.py` runs the same formatting, strict Clippy, locked tests
and warning-free documentation checks used by native CI. Use Python 3 (`python3`
on Macs without a `python` alias); Make is an optional convenience. Clippy enables
all, pedantic, nursery and cargo groups plus explicit panic, unwrap, indexing and
debug-output restrictions. Cognitive complexity is capped at 10; functions retain the 100-line and
six-argument limits. A syntax-aware test enforces nesting <= 2 inside each Rust
function/method and bans `else` (including let-else) in source and tests, including
Windows-only code. Negative fixtures verify the style checker rejects violations.
Scoped exceptions carry reasons; the few Clippy suggestions that require else
syntax are locally waived to honor the no-else rule.

Every PR also checks the declared Rust 1.89 minimum and runs `cargo audit --deny
warnings` with no advisory ignores. Install `cargo-audit` to run `make audit`
locally. Optional manual workflows produce native macOS/Windows LCOV coverage
and macOS mutation reports; they are audits, not claimed coverage or mutation
score gates. Their local equivalents require `cargo-llvm-cov` plus LLVM tools, or
`cargo-mutants`. Workflow success does not imply branch protection is configured.

## Use

```sh
nmpool scan --repo /path/to/repo --json
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
Preparation staging, npm download caches, and logs are retained under `CACHE/staging`
on success or failure; failed npm runs retain stdout and stderr in `install.log`.
Failed restore staging is retained under the destination package. These are for inspection;
they can consume disk space. There is no automatic cleanup. Never point the cache at
an existing dependency tree or an unrelated nonempty directory.

See [design and threat boundary](docs/design.md) and [validation](docs/validation.md).
