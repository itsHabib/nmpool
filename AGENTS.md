# nmpool

This is a personal Rust tool, independent of Fleet and of any employer implementation.
Read README.md and docs/design.md before changing behavior.

## Contract

- Every restored install is private. No consumer-to-cache hardlinks or junctions.
- Default to refusal on unknown provenance, unsupported inputs, incomplete reads,
  pre-existing destinations, reparse points, or ambiguous filesystem identity.
- No adoption of existing node_modules and no garbage collection in the first release.
- Keep native filesystem details in platform.rs; keep CLI and policy separate.
- Preserve unrelated work. Implement in a secondary worktree.
- Validate with `python scripts/check.py` (Python 3): formatting, strict Clippy
  across all targets/features, locked tests, and warning-free documentation.
  CI also checks Rust 1.89 and audits dependencies without advisory ignores.
  Native macOS and Windows CI are required; cross-builds
  alone do not prove Windows behavior. Actual work-laptop benefit remains a separate check.
- Before publishing, inspect tracked files for local paths, secrets and unrelated artifacts.
- Do not merge; the operator owns merge authority and grants.

## Style

Early returns, small functions, explicit errors, minimal dependencies. No daemon or
plugin framework. Do not generalize beyond a measured consumer.

Keep Clippy all/pedantic/nursery/cargo and the selective restriction lints enabled.
Cognitive complexity must be <= 10. Rust source and tests must have no `else`
(including let-else) and at most two nested block scopes inside each function or
method, with the function body at depth zero. `tests/style.rs` parses source before
cfg filtering, so Windows-only code is checked on Mac too. Comments and literals
are not code. Refactor violations; do not suppress complexity or nesting checks.
The shared `python scripts/check.py` command enforces these rules in native CI.
Every
local lint exception needs a reason; do not disable a lint globally to clear one
finding. Keep panic/unwrap exceptions confined to tests and stdout to the CLI.
Coverage and mutation workflows are optional, manually dispatched audits.
