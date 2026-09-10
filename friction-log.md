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

## 2026-09-09 — Windows trial and review bootstrap

- Added a retained-evidence, empty-package smoke command shared by macOS and Windows CI. Initial Mac rehearsal correctly refused `/var` (a symlink); canonicalize the new fixture root before invoking nmpool. No refusal bypass.
- Added the portfolio Claude workflow. Repository Actions secrets were empty; it requires operator credential setup and default-branch installation before comment triggers work. A local Claude review is explicitly separate evidence for the initial PR.
- Addressed current Codex findings: status locks before absent/untracked reports, census skips case-variant generated directories, and only a typed NotFound qualifies a missing lock as unsupported.

- Local Claude review of 913a316 identified a Windows short-name cache alias gap and ambiguous `explain --against-node` npm fallback; both addressed. Permission probes and census exclusions are now described precisely. Concurrent first cache initialization can still fail closed with a misleading incomplete-cache/AlreadyExists error; tracked as a liveness limitation, never permission to delete or adopt. Mutable CI action refs are existing policy debt; no new required-check or toolchain pinning policy is imposed in this trial patch.

## 2026-09-09 — Enforce operator strict style

The inherited limits allowed cognitive complexity 20 and did not enforce the
operator's nesting/no-else preferences. Set Clippy cognitive complexity to 10;
added syntax-aware source/tests checks for two nested scopes inside each function
and no else tokens, with negative fixtures and Windows cfg coverage. Refactored
violations rather than suppressing nesting/complexity. Clippy's own excessive
nesting counter includes function/impl containers, so the source parser defines
the intended function-relative boundary. Only conflicting let-else suggestions
have documented local exceptions. Existing lint groups, strict warnings, audit,
formatting and docs checks remain enabled.

Final Codex review of the prior head also identified unchecked linked ancestor
manifests and inspect accepting a cache under node_modules. Added both guards and
regressions while refactoring the touched code.
