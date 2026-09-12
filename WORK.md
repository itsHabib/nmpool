# Live sharing and adoption

Objective: complete opt-in shared node_modules from docs/features/share-adopt/spec.md
and issues #6, #7, #8 and #10. Qualification commands alone do not complete this work.
Branch: feat/live-sharing. PR: https://github.com/itsHabib/nmpool/pull/16

## Implemented and locally verified

- Explicit island policy supports npm v2/v3, generator bytes, approved staged scripts,
  declared workspace context, native runtime, private registries and local attestation.
- registry_hosts admits lockfile and npmrc hosts. Protected provenance.json preserves
  source hosts, supplied integrity, observation time, trust domain and manifest digest.
  The bounded artifact/v2 header binds its digest; a 3,000-source fixture checks the
  large record stays outside the header and tampering quarantines the generation.
- Multiple consumers attach to one protected physical tree. Fast checks use bounded
  headers/probes/current inputs; they explicitly do not claim full current verification.
  Concurrent approved runtime commands use separate writable state. Runtime acquisition
  waits for the pool holder without the previous arbitrary 30-second timeout.
- Planned adoption leaves originals untouched. Qualification requires the exact completed
  adoption candidate; another candidate's qualification receipt cannot be reused.
- Replacement retains original identity/content/provenance. Recovery preserves new user
  work and restores the original even when uncertain staging must be held separately.
- Durable local link intent precedes creation; interrupted pre-publication links have a
  recovery path. Successful build/qualification staging is removed without following
  links or changing published/retained trees. Failed staging remains held; no GC exists.
- Public CLI tests cover prepare/link/run/status (exit 2), adopt/qualify/planned replace,
  retained listing and rollback. Private-copy tests remain intact.
- Full local checks pass: 92 tests, formatting, strict Clippy and docs. Windows cross-target
  strict Clippy passes. Evidence: /private/tmp/nmpool-policy-chain-final.log and
  /private/tmp/nmpool-policy-chain-windows-final.log.

## Native evidence and remaining work

Native Windows passed 1b4c4ae. At 5848c41, run 34610124586 failed replacement with
ERROR_INVALID_PARAMETER from the RootDirectory relative-name variant. Current code
restores the previously working full-path rename while holding no-delete handles on
all destination ancestors, validating the parent's identity. A native-only test checks
ancestor rename is blocked while guards live. This fix still requires native CI.

Local Claude reviewed c678f27 and found a rollback/staging coupling, now covered by
the lost-target regression. The latest policy/provenance/candidate/native changes need
independent exact-head review. Copilot findings 3989940574, 3989940781 and 3989940828
are implemented; do not resolve them until native checks and review verify this head.
Hosted Codex review quota and GitHub Claude credentials were unavailable previously;
use current live evidence and the documented local Claude fallback.

Remaining deliverables: native macOS/Windows CI, final review/findings disposition,
Gate-authorized exact-head merge, public README/onboarding/build and work-laptop
handoff refresh. Actual work-laptop application acceptance/performance still needs
its sanitized profile/commands and reports (issue #10); do not invent those results
or let their absence stop portable implementation. Preserve original scope and keep
the goal active until completion is proven.


## Latest verification and outcome handling

Native Windows run 34611281589 passed at be9ae1c, including destination ancestor
guards, large provenance, candidate binding, sharing and recovery. Local Claude
review of be9ae1c reconciled to no P1/P2 (the conditional SHA1 finding was disproved
by capture validation and a public CLI refusal before cache creation). The fe84dbb
validation/docs delta independently received no P1/P2 findings.

Subsequent Cursor findings 3990293959/3990293972 are addressed by retrying the same
recorded adoption candidate with full source/candidate checks and by reporting a
cleanup_warning separately from successful publication/qualification. The adoption
receipt-interruption regression and a real Unix permission-denied cleanup CLI test
pass. Full local checks now pass with 94 tests, plus strict Windows cross-Clippy.
Latest evidence: /private/tmp/nmpool-outcome-check2.log and
/private/tmp/nmpool-outcome-windows.log. This outcome-handling revision still needs
independent delta review and native CI before Gate/merge. No goal completion claimed.
