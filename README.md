# nmpool

Prepare npm dependencies once. Reuse `node_modules` across Git worktrees with
protected shared generations or verified private copies.

[![CI](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Install

macOS or Windows · Rust 1.89+ · Git · native Node/npm

```sh
git clone https://github.com/itsHabib/nmpool.git
cd nmpool
cargo install --path . --locked
```

Not published on crates.io. [Windows setup](docs/windows-trial.md).

## Share dependencies

Copy and edit [an island profile](examples/island.json) for your package's registry
hosts, generator inputs, install/check commands and private runtime caches.

```sh
nmpool prepare --package /repo/web --cache /pool --profile /repo/island.json
# Use artifact_id from the output; the destination must have no node_modules.
nmpool link --package /worktree/web --cache /pool --profile /repo/island.json --artifact ARTIFACT_ID
nmpool run --package /worktree/web --cache /pool --profile /repo/island.json --tool check
```

Sharing supports npm lockfiles v2/v3, approved lifecycle scripts and declared HTTPS
private registries. Dependencies stay protected; writable tool caches live in each
consumer. Fast attachment checks are not a fresh full-content audit. Existing installs
can be adopted and replaced through explicit plans that retain the original.
See [shared installs, adoption and recovery](docs/live-sharing.md).

## Private copies

For npm v3 lockfiles with integrity-pinned public dependencies and no lifecycle
scripts or workspaces, omit the profile:

```sh
nmpool prepare --package /repo/web --cache /pool
nmpool restore --package /worktree/web --cache /pool
```

Each restored install has independent writable files. Existing installs are never
overwritten. Run the application's tests after either workflow.

Experimental. Real-workload correctness and performance need their own trial;
see [validation](docs/validation.md). Start another agent with
[agent onboarding](docs/agent-onboarding.md) or read the [command reference](docs/commands.md).

[MIT License](LICENSE) © 2026 Michael Habib.
