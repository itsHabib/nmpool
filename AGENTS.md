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
- Validate with cargo fmt --check, cargo clippy --all-targets -- -D warnings,
  and cargo test --locked. Native macOS and Windows CI are required; cross-builds
  alone do not prove Windows behavior. Actual work-laptop benefit remains a separate check.
- Before publishing, inspect tracked files for local paths, secrets and unrelated artifacts.
- Do not merge; the operator owns merge authority and grants.

## Style

Early returns, small functions, explicit errors, minimal dependencies. No daemon or
plugin framework. Do not generalize beyond a measured consumer.
