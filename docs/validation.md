# Validation

## Functional checks

Local macOS checks pass: cargo fmt --check, strict Clippy and 14 contract tests.
The tests include a native Node/npm prepare/restore/inspect round trip without
network dependencies. Native macOS and Windows CI are required on the PR head.
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
actual Windows-laptop performance, ReFS acceleration and independent review remain
unproven. Keep the PR experimental. No merge or GC is authorized by these results.
