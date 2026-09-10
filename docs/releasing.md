# Releasing nmpool

The first public release is an experimental, source-installable CLI for macOS
and Windows. The package is deliberately marked `publish = false`: a GitHub
release does not publish it to crates.io.

## Candidate checks

Use a clean checkout of the exact candidate commit and record its SHA.

```sh
git rev-parse HEAD
python3 scripts/check.py
cargo build --release --locked
python3 scripts/smoke.py --binary target/release/nmpool
```

On Windows use `python` and `target/release/nmpool.exe`. Require native macOS and
Windows CI, minimum Rust version checks and dependency audit to pass for that
candidate. Inspect completed review findings for the same head. Passing checks
prove neither real-workload benefit nor freedom from bugs.

Run a full-history secret scan before changing repository visibility:

```sh
gitleaks detect --source . --no-banner --redact
```

Inspect tracked files and the history that will become public for personal paths,
credentials, employer references and unrelated artifacts. Inspect the public
GitHub surface too: issues, PR conversations and Actions logs/artifacts. A clean
secret scan is one check, not a guarantee that every historical detail is suitable
for publication. Confirm the MIT license and package metadata still agree.

## Publish after review

The maintainer chooses the final merged commit. Date the changelog for the actual
release, tag that exact commit as `v0.1.0`, and create a GitHub prerelease using
[the prepared release notes](release-notes-v0.1.0.md). Review a draft before
publishing. Repository visibility, tag creation and release publication are
separate maintainer actions; this guide does not perform them.

Suggested repository description:

> Experimental verified private npm install cache for macOS and Windows.

The source-install instructions work from a clone. For a versioned install after
the tag exists, check out `v0.1.0` before `cargo install --path . --locked`.
Do not advertise `cargo install nmpool` while the crate is unpublished.

CI artifacts expire after 14 days. They are trial binaries, not permanent release
assets. If attaching binaries to a release, use successful native builds from the
exact tagged commit, identify the architecture, smoke-test them on their native
platform, and attach SHA-256 checksums. Never label an older build as the new tag.

## Acceptance after launch

Follow [the Windows trial guide](windows-trial.md) for a supported real package.
Keep compatibility, application correctness and timing results separate. The
existing small-package Mac result was slower than warm npm; no acceleration or
physical storage savings are claimed by this release.
