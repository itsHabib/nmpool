# nmpool

Prepare npm dependencies once. Restore a verified, private install into each Git worktree.

[![CI](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

When you work across branches—or give coding agents their own worktrees—each
checkout needs dependencies. nmpool builds a local cache from your package inputs
and restores a separate writable `node_modules` into a fresh checkout. It records
what was restored, so you can check whether inputs or installed files have changed.

Installs are matched by package inputs, configuration and runtime, not branch
names. Two branches can reuse the same entry; different Node/npm versions or
package inputs select different entries. Every consumer gets its own files.

## Install

Requires **Rust 1.89+**, **Git**, and native **Node/npm** for dependency operations.
Supports macOS and Windows; use native PowerShell on Windows.

```sh
git clone https://github.com/itsHabib/nmpool.git
cd nmpool
cargo install --path . --locked
nmpool --help
```

Install from source; nmpool is not published on crates.io. Windows uses
`nmpool.exe`. If the command is not found, add Cargo's `bin` directory to PATH
(`~/.cargo/bin` on macOS, `%USERPROFILE%\.cargo\bin` on Windows).

For a quick check with no dependency downloads, run this from the nmpool checkout
with Python 3 installed (`python` on Windows):

```sh
python3 scripts/smoke.py --binary nmpool
```

The script creates a disposable package and checks preparation, private restore,
overwrite refusal, drift detection and cache integrity. It prints the location of
its retained results.

## Use it in a worktree

Choose a supported package and a fresh worktree with no `node_modules`. Replace
the paths below with your repository, package directories and a new cache directory.
For a repository-root package, the package path is the worktree root.

```sh
# Find packages and their supported/unsupported status.
nmpool scan --repo /path/to/repo --json

# Build the cache without changing the source package's install.
nmpool prepare --package /path/to/repo/package --cache /path/to/nmpool-cache

# Restore into a fresh worktree, then check the installed files.
nmpool restore --package /path/to/fresh-worktree/package --cache /path/to/nmpool-cache
nmpool status --package /path/to/fresh-worktree/package
```

Run the package's own tests or build in the restored worktree. Keep using the same
cache on that machine for subsequent matching worktrees. Cache entries are specific
to their platform and runtime.

To compare two worktrees before installing:

```sh
nmpool explain --package /path/to/first/package --against /path/to/second/package
```

`status` exits **0** for clean, **2** for absent/untracked/drifted, and **1** for an
error. Existing npm installs are untracked; nmpool does not adopt or replace them.
A cache miss requires `prepare`; `restore` never silently runs npm.

## What it supports

The current release supports **npm package-lock v3**, integrity-pinned public npm
registry dependencies, and packages without install scripts or workspaces.
Project `.npmrc` may set `legacy-peer-deps`; other custom configuration, private
registries, local/Git dependencies and other lockfile formats are refused.

Restores are fully verified private copies. There are no shared writable installs,
automatic worktree hooks, adoption or garbage collection. Preparation logs and
staging files are retained, so follow the [cleanup guide](docs/windows-trial.md#troubleshooting-and-cleanup)
after disposable trials.

## Give it to an agent

Start with **[Agent onboarding](docs/agent-onboarding.md)**. It includes a ready-to-paste
prompt, the first-run sequence, failure handling, evidence to return, and a code map
for agents contributing to nmpool itself.

## Status

**Experimental and usable.** Native macOS and Windows CI exercise the CLI, and
real application tests have passed after restore. A successful work-agent run
has also been reported by the maintainer. See [validation](docs/validation.md)
for the evidence and its scope.

Performance depends on the workload. The measured small-package Mac trial restored
in 1.189 seconds versus 0.425 seconds for warm npm ci; no general speedup or disk
savings are claimed. Broader package support and shared installs are described in
[future direction](docs/direction.md).

## Documentation

- [Command reference](docs/commands.md): command behavior, receipts and exit codes.
- [Windows walkthrough](docs/windows-trial.md): installation, real-package trial and troubleshooting.
- [Design](docs/design.md): cache identity, filesystem behavior and concurrency.
- [Agent onboarding](docs/agent-onboarding.md): using and extending nmpool.
- [Changelog](CHANGELOG.md) and [release guide](docs/releasing.md).

## License

[MIT](LICENSE) © 2026 Michael Habib.
