# Agent onboarding

Use this guide when adding nmpool to a project's worktree workflow or when
contributing to nmpool itself. You do not need context from an earlier conversation.

## Use nmpool in another repository

Collect five inputs before installing: repository root, package path relative to
that root, source revision, a new destination worktree path, and a dedicated cache
path. Read the consuming repository's instructions and find its actual test/build
command. Preserve existing installs and uncommitted work.

1. **Check the tool.** Follow the [installation instructions](../README.md#install).
   Record the nmpool commit or binary provenance, OS, architecture, Node and npm
   versions. Run `scripts/smoke.py` from the nmpool checkout on a new machine.
2. **Check the package.** Run `nmpool scan --repo REPO --max-depth 4 --json`.
   Read the package row's `unsupported_reason` and any scan errors. A supported
   candidate is eligible for preparation. If unsupported, report the exact reason;
   do not rewrite its scripts, lockfile or registry configuration to pass.
3. **Create the consumer.** Use the repository's worktree convention to create a
   fresh checkout at the intended revision. Include the complete source needed by
   its tests, including shared scripts and fixtures. Confirm the destination
   package has no `node_modules` and no installer or watcher is using it.
4. **Prepare and restore.** Run `prepare` against the source package and `restore`
   against the destination package using the same dedicated cache. First use
   `explain` to confirm matching requirements; if the source checkout differs from
   the selected revision, prepare against the new worktree package instead. Save
   both JSON outputs. Preparation installs into cache staging, not the source package.
5. **Check correctness.** Run `status` in the destination; expect clean/exit 0.
   Run the application's own tests/build there, then run `status` again. Tests
   that mutate dependencies may produce drift; report it rather than rewriting
   the receipt. Use `inspect --cache CACHE --key KEY_FROM_PREPARE` to check the seed.
6. **Hand back a usable result.** Report the destination and cache paths, source
   and tool revisions, commands/results, and any remaining work. Keep the worktree
   if it is the requested working environment. Remove only trial-owned worktrees
   when cleanup was requested; retained cache/staging files need separate cleanup.

Native Windows examples, checks for command failure, and a timing protocol are in
[the Windows walkthrough](windows-trial.md). On macOS the same CLI arguments use
ordinary POSIX paths. Quote paths containing spaces. Cache roots and destinations
must satisfy nmpool's plain-path checks; do not bypass symlink/reparse refusals.

## Paste-ready task prompt

Replace the bracketed values before giving this to an agent:

```text
Use nmpool to prepare dependencies for a new worktree of [repository].
Package subdirectory: [path relative to repository root].
Source revision: [commit or branch to resolve and record].
Destination worktree: [new path]. Dedicated cache: [new or nmpool-owned path].
Application validation command: [test/build command, or discover from repo docs].

Read the repository instructions and nmpool's docs/agent-onboarding.md. Check
package support, create the fresh worktree, prepare the cache, restore, check
status, and run the application's validation command. Preserve existing installs.
Stop on unsupported inputs or corrupt/busy state and report the exact reason.
Do not delete installs, alter lockfiles/scripts to gain support, or bypass locks.
Return revisions, runtime versions, destination/cache paths, command exit codes,
application test results, and remaining work. Leave the new worktree ready to use.
```

## Interpret results

| Result | Next action |
|---|---|
| `status`: clean, exit 0 | Run/use the application; this is an install snapshot check. |
| `status`: absent, exit 2 | Restore if this is the intended fresh destination. |
| `status`: untracked, exit 2 | Preserve it; use a fresh worktree for a recorded restore. |
| `status`: drifted, exit 2 | Report input/file differences; choose a fresh destination for changed inputs. |
| `scan`: exit 2 | Inspect partial-scan errors; missing worktrees are not evidence of no consumers. |
| `explain`: exit 0 | Inspect `same_install_requirements`; exit 0 does not mean the keys match. |
| `destination_exists` | Use a fresh destination; do not remove an install to make restore pass. |
| `cache_miss_or_incomplete` | Check package/runtime identity and preparation evidence; preserve incomplete entries. |
| `cache_busy` / `destination_busy` | Let the current operation finish; do not delete locks. |
| `npm_install_failed` | Read the reported staging `install.log`; report a sanitized failure reason. |
| Artifact mismatch or corrupt receipt | Preserve the evidence and stop using that entry. |

Never edit `.nmpool-restore.json`. It records inputs and installed artifacts, not
which agent changed a file. There is no automatic repair, adoption or GC command.
For speed claims, follow the timing protocol and compare the same package/runtime
and install recipe; a successful restore alone is functional evidence.

## Contribute to nmpool

Read [AGENTS.md](../AGENTS.md), [design.md](design.md), and the relevant command
reference first. Work in an isolated worktree and preserve the private-copy
contract. [direction.md](direction.md) describes proposals, not implemented verbs.

| File | Responsibility |
|---|---|
| `src/main.rs` | CLI arguments, command dispatch and exit codes. |
| `src/inputs.rs` | Supported package profile, input fingerprints and Node/npm identity. |
| `src/cache.rs` | Cache ownership, locking, preparation, receipts and restoration. |
| `src/tree.rs` | Artifact manifests, hashing and verified private copies. |
| `src/platform.rs` | Native paths, file identity/copying and atomic publication. |
| `src/state.rs` | Restoration records, drift reports and requirement comparisons. |
| `src/census.rs` | Bounded inventory of Git worktrees and package candidates. |
| `tests/contracts.rs` | Behavioral and filesystem regression checks. |
| `tests/style.rs` | Rust style constraints, including Windows-only source. |

Run `python3 scripts/check.py` (`python` on Windows): formatting, strict Clippy,
locked tests and warning-free docs. Build the release binary and run the smoke
script when changing CLI behavior. Native macOS and Windows CI are required;
a cross-build does not substitute for native behavior checks.

Open a focused PR with the problem, behavior and validation evidence. Follow
[the review process](reviews.md). Hand off the exact commit, completed checks,
review findings and unresolved limitations; do not merge without maintainer
authority. Publishing instructions live in [releasing.md](releasing.md).
