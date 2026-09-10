# nmpool v0.1.0 — experimental preview

nmpool is a local dependency-install cache for macOS and Windows. Prepare an
install once, then restore a verified private copy into a fresh worktree. Existing
installs are never adopted, replaced or deleted.

Commands: `scan`, `prepare`, `restore`, `inspect`, `status`, and `explain`.
Restoration records let you detect changed inputs, runtime or installed files.

## Try it

After the `v0.1.0` tag is published:

```sh
git clone https://github.com/itsHabib/nmpool.git
cd nmpool
git checkout v0.1.0
cargo install --path . --locked
nmpool --help
```

Requires Rust 1.89+ and Git; prepare/restore also need native Node/npm.
See the [README](https://github.com/itsHabib/nmpool/blob/v0.1.0/README.md) and [Windows trial guide](https://github.com/itsHabib/nmpool/blob/v0.1.0/docs/windows-trial.md).
This release is installed from source and is not on crates.io.

## Scope and evidence

- npm lockfile v3 and integrity-pinned public registry dependencies only.
- No install scripts, workspaces, private registries, shared writable installs,
  automatic worktree integration, adoption or garbage collection.
- Native macOS and Windows checks exercise the implementation. Run your
  application's own tests after restoring dependencies.
- No speedup or disk savings are claimed. The measured small Ivy package restored
  in 1.189 seconds versus 0.425 seconds for warm npm ci. Large private monorepos
  remain outside the supported profile.

Failed staging and preparation logs are retained for diagnosis and can consume
disk space. Use a disposable trial first and follow the guide's cleanup steps.
