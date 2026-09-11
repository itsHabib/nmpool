# nmpool

Prepare npm dependencies once. Restore a verified, private `node_modules` into each Git worktree.

[![CI](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

For developers and coding agents working across branches. Each worktree gets its
own writable files; existing installs are never replaced.

## Install

macOS or Windows · Rust 1.89+ · Git · native Node/npm

```sh
git clone https://github.com/itsHabib/nmpool.git
cd nmpool
cargo install --path . --locked
```

Not yet published on crates.io. See the [Windows guide](docs/windows-trial.md) for
PowerShell instructions and binary downloads.

## Use

Replace these paths with your repository, package directories and a dedicated
cache. The destination package must have no `node_modules`.

```sh
nmpool scan --repo /path/to/repo --json
nmpool prepare --package /path/to/repo/package --cache /path/to/cache
nmpool restore --package /path/to/new-worktree/package --cache /path/to/cache
nmpool status --package /path/to/new-worktree/package
```

Run your package's tests after restoring. Reuse the cache for matching package
inputs and Node/npm versions on the same machine.

## Supported scope

The intended tool supports private copies and shared `node_modules` across worktrees.
Opt-in sharing, adoption and generator-aware installs are implemented on this
experimental branch; see [shared installs](docs/live-sharing.md) for the explicit
policy and commands. Native validation and review are required before release.

npm lockfile v3, integrity-pinned public registry dependencies, no install scripts
or workspaces. Private registries, local/Git dependencies and custom npm configuration
are unsupported, except project `legacy-peer-deps`.

Working toward sharing? Run the [qualification checks](docs/sharing-trial.md).

Experimental and usable. No general speedup or disk savings are claimed;
see [validation](docs/validation.md) for results.

## Docs

- **[Agent onboarding](docs/agent-onboarding.md)** — paste-ready prompt, workflow and code map.
- [Command reference](docs/commands.md) — receipts, exit codes and detailed behavior.
- [Design](docs/design.md) · [Changelog](CHANGELOG.md)

[MIT License](LICENSE) © 2026 Michael Habib.
