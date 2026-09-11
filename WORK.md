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
