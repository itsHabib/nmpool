# Live sharing and adoption

Objective: complete the opt-in shared node_modules workflow described in
`docs/features/share-adopt/spec.md` and issues #6, #7, #8 and #10. Private copy
remains supported. Qualification commands alone do not complete this work.

## Completion evidence

- Versioned per-island policy keys npm v2/v3, generator input bytes, approved
  recipe, workspace context and native runtime; private registries and missing
  upstream integrity retain explicit local provenance.
- Staged script/generator installation publishes protected, fixed generations.
- Two consumers attach to one generation without copying/full scanning on the
  fast path; required probes, current request and physical identities are checked.
- Approved runtime commands receive separate writable cache directories; dependency
  bytes remain unchanged after concurrent commands.
- Planned adoption copies and verifies an existing tree without moving it;
  qualification binds the exact candidate and inputs to application checks.
- Explicit replacement retains original tree with durable transaction provenance.
  Recovery returns only that exact tree and never overwrites new user work.
- Pool aliases, source swaps, missing/empty targets, held handles, collisions,
  corruption and interrupted transaction transitions preserve seed/retained data.
- Current private-copy suite plus new public CLI tests pass locally and on native
  macOS/Windows. Independent reviews and Gate authorize the exact merged head.
- README and onboarding show usable commands and accurate verification limits.

Actual work-laptop application acceptance and performance require its reports.
Do not invent those results or use their absence to stop portable implementation.
No automatic garbage collection or published-generation mutation is introduced.

Branch: feat/live-sharing. Implementation files are split among bounded workers;
parent integrates CLI, end-to-end tests, documentation, CI and review.

## Review follow-up, September 11

Current follow-up fixes legacy v1 plan parsing, disposable build/qualification
cleanup, recovery preview validation, durable package-local link intent/recovery,
README mode wording, and full-audit quarantine classification. Public CLI coverage
now includes adopt/qualify/planned replacement/retained/rollback. Local full checks
pass (89 tests); Windows cross-target Clippy passes. Native CI still must verify
this revision. Previous published head 1b4c4ae passed native Windows.

Remaining review disposition: Copilot comments 3989940574 (trust-domain meaning),
3989940781 (candidate qualification
binding), 3989940828 (provenance). Also assess suppressed lock-wait feedback and
finish shared-status CLI exit coverage. Review source is PR16 and local review
reports; no final approval or merge is claimed. Gate and work-laptop handoff remain.

Second follow-up pins Windows rename to a verified destination-parent handle and
keeps retained-original rollback independent of staging-target availability
(`staging_held` reports the leftover). Local full checks and Windows cross-Clippy
pass; native verification of these changes remains required. Local Claude reviewed
c678f27 and found the staging/rollback coupling; the new lost-target regression
covers its fix. Its two P3 observations (empty pre-intent directory residue and
missing recovered marker on an unpublished attachment) remain non-destructive.

Next implementation requirement from spec lines 142-172: explicit registry-host
policy and immutable local-attestation provenance including registry identity,
observation time and preserved upstream integrity. Current `trust_domain` is only
a namespace and policy hash, so do not call this requirement complete. Keep the
bounded header; detailed provenance must not overflow it on the large real island.
