# Validation

## Functional checks

Local macOS checks pass: cargo fmt --check, strict Clippy and the contract suite.
The tests include a native Node/npm prepare/restore/inspect round trip without
network dependencies. Regression checks cover failed-install log retention and
scan/census alias equivalence, atomic restoration records, unknown/corrupt records,
input/runtime/file drift, comparisons across branches, and cache-independent status. Native macOS and Windows CI are required on the PR head.
The first Windows run found external-tool path handling defects; the patch uses
safe Windows path simplification. Exact CI status is linked from the PR checks.

Read-only census ran over personal Ivy and Roxiq worktrees: 224 package paths,
three missing registered worktrees, explicit partial-scan exit 2. This is a bounded
snapshot, not an exhaustive portfolio inventory or a deletion plan.

## Disposable Ivy MCP trial

Measured implementation commit: ed6b53b1ca390b8f41196b2d5c9b7948d9019cac.
Personal Ivy source: 60f8bcc3c787668e196ec7dc0251c9f3b3394385.
Machine: macOS 26.6.2, arm64/APFS, Node 26.5.1, npm 11.17.0, release build.
Later hardening changes have separate contract/CI checks; these timings are not
represented as measurements of a later commit.

| Operation | Wall time |
|---|---:|
| Fresh staged preparation, initially empty task cache | 1.720 s |
| Warm npm ci, three runs | 0.425 / 0.426 / 0.390 s |
| Verified restore, three runs | 1.446 / 1.188 / 1.189 s |
| Warm install median | 0.425 s |
| Verified restore median | 1.189 s |

Wall time wraps the full CLI, including runtime fingerprinting and validation.
Baseline used the same scripts-disabled recipe and the npm download cache populated
by preparation. Runs alternated warm install and restore in one disposable worktree.

- Ivy's actual 9 MCP tests passed after both baseline and restored installs.
- Editing a restored dependency left the seed intact; full cache inspection passed.
- Moving the one disposable worktree, restoring again, and running all 9 tests passed.
- Git removed the disposable worktree; the cache still verified identically.
- No live dependency tree was replaced or adopted. Task-owned cache and logs were
  retained; physical extent savings were not measured.

**The speed hypothesis failed for this small consumer.** Verified restoration was
about 2.8 times slower than warm npm ci. Do not deploy broadly or remove integrity
checks on the strength of this trial. Roxiq install scripts remain unsupported;
end-to-end Windows-laptop performance and ReFS acceleration remain unproven.
Later component measurements and unsupported-profile findings are recorded in
[direction.md](direction.md). Keep the release experimental. These trial results
do not authorize GC.


## Remaining release work

- Implementation PR #1 and launch-prep PRs #2 and #3 have merged. Review evidence
  belongs to their recorded heads; see the PR discussions and [review process](reviews.md).
  CI is not a bug-free claim.
- Run the [Windows trial](windows-trial.md) against a supported real package.
- Coverage and mutation workflows are configured but have not been run.
- Main branch protection is not configured. Merge remains operator-controlled.
- Script/workspace/private-registry support requires a separate design; it is not
  part of this trial. Physical savings and acceleration are still unmeasured.


## Restoration-record trial

A fresh task-owned snapshot of the same Ivy revision (MCP, shared scripts and its
required course fixture) passed all nine MCP tests after restore. Status was clean
both before and after the application tests. Appending a newline to the private
SDK package manifest produced exit 2 and named that exact modified file; inspection
of the cached seed still passed. The initial reduced snapshots omitted test fixture
dependencies and failed until those source files were included. No live install
was changed. This is functional evidence, not a new speed measurement.

## Maintainer-reported work-agent run

On 2026-09-10, the maintainer reported that an agent had run nmpool successfully
in their work environment. This is an additional successful-use report beyond
the recorded local fixtures and CI. The exact tool revision, package, commands,
application test results and timings were not supplied with that report. It does
not change the supported-input profile or establish a new performance result.
