# Tooling friction

- 2026-09-08: an early cargo check ran before the census module had been written.
  Re-ran after implementation; no platform result was inferred from that failed build.
- 2026-09-08: parallel subprocess tests exposed Unix flock lifetime around fork/exec.
  Cache Drop now explicitly unlocks in addition to closing the descriptor. Native
  lock tests verify exclusion and immediate release.
- 2026-09-08: portfolio census found missing registered worktrees. The command emits
  partial-scan exit 2 and retains errors; no worktree prune or repair was attempted.
- 2026-09-08: native Windows CI rejected Rust's verbatim canonical paths in Git
  and Node (EISDIR / invalid argument). Used dunce canonicalization to retain
  filesystem identity while simplifying representable paths for external tools.
