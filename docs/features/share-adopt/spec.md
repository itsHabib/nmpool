# Shared installs and adoption — Technical Design Document

**Status:** draft for design review; commands below are proposed, not implemented.
**Owner:** @itsHabib · **Date:** 2026-09-10
**Related:** [#10](https://github.com/itsHabib/nmpool/issues/10),
[#6](https://github.com/itsHabib/nmpool/issues/6),
[#7](https://github.com/itsHabib/nmpool/issues/7),
[#8](https://github.com/itsHabib/nmpool/issues/8), [current contract](../../design.md).

> Review focus: what fast verification can honestly guarantee; writable state
> behind junctions; adoption and rollback through crashes; evidence needed to
> admit the actual Windows package rather than merely relax six checks.

## 1. Problem and hypothesis

The intended product is a Rust successor to the Go junction-sharing workflow:
reuse one installed dependency tree across worktrees, adopt useful existing
installs, and distinguish generator inputs such as Prisma schemas. Private copy
mode remains useful and retains its current defaults and contract. This proposal
changes the future direction; it does not claim the current Rust CLI implements it.

Issue #10 reports a 1.9 GiB, approximately 109k-file install across approximately
130 worktrees: junction creation took 99 ms, unverified copying 48 s, and one full
hash 254 s. These are reporter measurements, not independently reproduced Rust
benchmarks. The motivating application is inside a pnpm monorepo but reports an
npm v2 lock for its package; its actual install boundary must be established.

Hypothesis: a qualified package can share fixed dependencies while isolating
runtime writes, with attach latency independent of dependency-file count. Full
verification moves to publication and explicit audits. That sacrifices continuous
integrity assurance, which must be visible rather than hidden behind “verified.”

Non-goals for the first trial: porting unpublished employer code, general pnpm
support, transparent filesystem overlays, automatic GC, cross-machine caches,
or an administrator/service-based security boundary. Implement personal Rust code
from the requirements and sanitized reproductions.

## 2. Requirements

- Retain `prepare` and private `restore`; existing copy receipts and caches remain
  governed by their recorded schema without silently acquiring weaker verification
  semantics. #9 moves the current input schema to v2; v1 entries are preserved but
  require the older binary for inspection and fresh preparation for current use.
- Add explicit share and adoption profiles for a qualified installation island.
  An island is a declared package/install boundary, not merely a directory with a
  matching lockfile. No automatic exemption for all packages in a monorepo.
- Pin generator sources, configuration, toolchain and install recipe in requested
  identity. Record built artifact identity separately.
- Every shared consumer points to one exact artifact generation; regeneration
  creates another generation and never rewrites a published tree in place.
- Refuse attach on unknown identities, invalid/missing receipts, missing/empty
  required trees, failed probes, unresolved transactions, or known corruption.
- Preserve adoption rollback sources and report their provenance. Never infer a
  recovery match or deletion permission from file counts or age.
- `.cache` compatibility is a per-tool execution contract; concurrent worktrees
  must not accidentally write into a common cache.

| Concern | Trial acceptance target |
|---|---|
| Safety | Zero lost/modified seed files through every native Windows failure rehearsal |
| Correctness | Same recorded application checks pass against baseline and two shared consumers |
| Isolation | Parallel format/build/test runs do not modify dependency files or another consumer's writable state |
| Speed | Median complete attach < 2 s and < 10% of measured private restore on the actual machine, 10 runs each |
| Observability | Every attach reports verification tier, artifact ID and last full audit; every retained tree has a transaction identity |
| Compatibility | Existing private-copy contract suite passes on native macOS and Windows |

Failure of the speed target stops rollout pending investigation; correctness and
safety failures block share regardless of speed. Do not exclude identity/config
work or junction validation from timing.

## 3. Architecture

```text
island policy + inputs + runtime -> request key
             prepare/adopt -> staged tree -> full manifest -> artifact ID
                                                          |
                                                   published generation
                                                     /          \
                                             private copy    share attachment
                                                            + private runtime state
```

Reuse `inputs.rs` for input/toolchain capture, `tree.rs` for full manifests,
`cache.rs` for publication and existing cache locks, and `state.rs` for reports.
Introduce policy, transaction and attachment modules as the corresponding phases
land. All new native identity, junction and no-follow move primitives stay in
`platform.rs`; existing `plain_path` must not become globally permissive.

Copy continues to use the existing full verification path. Share entries live in
an explicitly versioned store namespace; copy entries cannot be attached merely
because their receipt parses. New profiles require new qualified receipts.

## 4. Decisions and trade-offs

### Fixed identity is not enforced immutability

An artifact manifest is immutable metadata. A junction to writable bytes is not
an immutable dependency tree. Readonly attributes and process convention alone
must not be described as a security boundary, particularly against the same user.

The first candidate is **managed immutability**: nmpool never changes a published
generation, users stop installers before attachment, approved commands redirect
writes, and full audits detect dependency drift. Report `protection=managed`,
not `immutable=true`. Shared writes remain a blast-radius risk between audits.
The user must accept that concrete limitation before a real shared trial.

Enforced immutability would require a separately tested filesystem/permission
boundary denying writes, creates, renames and deletion from consumer processes.
It is deferred, not claimed by the proposed fast checks. Private copy remains the
fallback when tools cannot honor managed immutability or stronger isolation is
required.

### A root junction cannot give each worktree its own `.cache`

Precreating `.cache` in the shared target could avoid #7's missing-directory error,
but it would give every consumer the same writable directory. A junction inside
that target also has one common destination. Neither provides per-worktree caches.

For the first share profile, qualify each actual tool command and direct caches
outside `node_modules` into `<package>/.nmpool-runtime/<consumer-id>/<tool>` using
that tool's supported configuration. Precreate these private cache directories.
Do not invent an environment variable or assume all tools support redirection.
A required tool with hardcoded writes under `node_modules/.cache` blocks this
root-junction profile; use private copy for that workload.

A later alternative is a private `node_modules` facade containing private `.cache`
and links to shared packages, with private executable shims where needed. That
changes module resolution/realpath behavior and needs separate compatibility and
performance trials. It is not a trivial overlay and is not part of phase 1.

Generator outputs required for imports, including Prisma clients, belong in the
artifact when invariant for the declared inputs. Outputs varying by consumer
belong outside the shared tree or force private materialization. No generator
runs against a published generation.

### Per-island inputs, scripts and private dependencies

The island policy is checked-in, versioned data. It declares package path,
package-manager/version, lock format, ancestor/workspace context, accepted config
keys, registry identities, exact generator input paths and install/generation
commands. The policy hash itself enters the request key.

Admitting an npm island inside a pnpm workspace requires proving its install does
not resolve undeclared siblings, inherited configuration or workspace links.
Otherwise the workspace closure becomes declared input or the profile refuses;
removing `reject_workspace` globally is not an implementation plan.

Input capture includes manifest/lock, relevant workspace manifests, allowlisted
nonsecret npm configuration (including `engine-strict` when qualified), generator
schemas and imports, generator versions/options, selected environment values,
Node/npm/platform identity and fixed recipe. Missing inputs differ from empty
ones; unknown reads fail. JSON line-ending normalization follows the separate #9
fix; arbitrary generator/binary files retain byte identity.

Scripts are executable code with external effects, not just additional key fields.
Run approved scripts in staging with controlled environment and declared writable
paths. Record commands, outputs and sanitized logs. No live database migration,
source mutation, or opaque shared-global-cache dependency is admitted implicitly.
A script requiring undeclared effects blocks publication. A staging directory is
not a sandbox; isolation adequacy is a profile review and native trial gate.
Never store credentials, tokens or credential-bearing URLs in receipts/logs. Use
an explicit private-registry credential channel; record a nonsecret trust-domain
identifier when authorization context can change resolution.

Missing upstream integrity does not become fabricated `integrity` data. Record
`origin=local-attestation`, the fetched/installed artifact's full hash, source
registry identity and observation time. This establishes the bytes observed
locally, not publisher authenticity or reproducibility. Preserve upstream
integrity separately when supplied. Two installs with one request key but different
artifact manifests are distinct generations; refuse an ambiguous automatic choice
and require an explicit artifact selection. Credentials never become the key.

### Adoption is a transaction, not a destructive shortcut

The existing tree must be a verified ordinary directory outside every pool entry,
attachment and pending transaction. Reparse points are classified without following
them. Resolved ancestor/file identities, volume IDs and handle identities establish
containment and sameness; string-prefix comparison is insufficient. Revalidate
through the final native move and refuse unresolved alias or race behavior.

A locally observed tree cannot prove that current lock/schema files produced it.
Adoption therefore produces a **candidate**, with a full manifest and explicit
local attestation. Promotion needs a qualified receipt from a controlled build,
or a reviewed adoption policy plus recorded application/generator validation.
Label the latter `locally-attested`; it is not silently equivalent to a fresh
controlled build. Existing unexpected links, undeclared generated contents or
unknown install provenance keep the candidate unqualified.

For the first adoption implementation, copy to staging, fully verify, then publish
an independent candidate; leave the original install intact as rollback. After
qualification, an explicit attach transaction may set aside the original using a
native identity-bound no-replace move. This one-time cost buys simpler recovery;
move-only adoption is a later optimization requiring its own failure evidence.

## 5. Data model

New versioned records (schema validation rejects unknown major versions):

- `IslandPolicy`: ID/version, package boundary, declared input paths, manager and
  recipe, registries/config allowlist, scripts/effects, runtime-write routing,
  qualification tests, permitted verification/protection modes.
- `ArtifactReceipt`: request key, artifact ID/full manifest, policy hash, runtime,
  generator input hashes, origin/upstream-integrity evidence, qualification
  evidence references, state, last full audit. Manifest IDs never contain paths.
- `Attachment`: consumer UUID, package identity, artifact ID, native link identity
  and target, runtime-state location, transaction ID, verification tier.
- `Transaction`: unique ID, operation, state, source/destination native identities,
  source package/worktree and Git context, request/artifact IDs, manifest hash,
  timestamps and retained-tree location. File count/bytes are diagnostic only.

Suggested layout: `shared-v1/artifacts/<artifact-id>/`, `transactions/<id>/`,
`retained/<id>/tree` plus `provenance.json`. Consumer records live outside the
shared tree. Paths from receipts are untrusted display data; operations resolve
validated IDs against known roots and confirm native identity. Retained trees
are not GC candidates in this release.

## 6. Proposed CLI contract

These are design interfaces, not commands available today:

```sh
nmpool prepare --package P --cache C --profile island.json
nmpool restore --package P --cache C                 # unchanged private copy
nmpool link --package P --cache C --artifact A --profile island.json
nmpool adopt --package P --cache C --profile island.json --plan
nmpool adopt --package P --cache C --plan-id T        # creates candidate only
nmpool inspect --cache C --artifact A --full
nmpool status --package P --verification structural
nmpool status --cache C --retained
nmpool recover --cache C --transaction T --plan
```

Plans bind input, source and destination identities plus artifact IDs; execution
revalidates them and refuses stale plans. No blanket `--force` or auto-repair flag.
Qualification and recovery execution syntax must be finalized in their phase specs.
A link against an existing ordinary install requires an explicit replacement plan;
an already matching link is idempotent only after target checks succeed.

Results report `operation`, `state`, `request_key`, `artifact_id`, `verification`,
`protection`, `last_full_audit`, `transaction_id`, and diagnostic paths. Success
means the named operation/tier passed, never application correctness. A structural
status is `attached_unverified`, not `clean`. Non-clean observations exit 2; invalid
records, missing/corrupt entries and incomplete reads exit 1. Errors include stable
codes such as `entry_missing`, `entry_corrupt`, `identity_changed`,
`adoption_unqualified`, `runtime_write_unsupported`, and `transaction_incomplete`.

## 7. Lifecycle and failure flows

**Publish:** `staging -> fully_verified -> qualified -> published`. A failed read,
input change, script failure or manifest mismatch retains staging and cannot
produce a published entry. No-replace publication pins one artifact generation.

**Attach:** lock cache then destination; capture the destination policy, lock,
generator closure and runtime and compute its current request key. Read the
selected published receipt and require its request key to match. `--artifact`
selects a generation within one request; it never overrides input compatibility.
Check exact tree and target identities plus structural probes; write prepared
transaction; build a link in private staging; recheck the captured inputs/runtime
and destination identity immediately before publication. Refuse `request_mismatch`
or `inputs_changed` rather than publishing on a mismatch. Publish only to an absent
destination; record committed attachment. Existing untracked directories and unknown links are refused. Crash
recovery reconciles the transaction with actual identities before retrying.

**Replace an existing install:** explicit plan; full source manifest and provenance
persist before any move; source identity rechecked without following reparse
points; move ordinary source to retained area on the same volume; publish checked
link to now-absent destination; commit. Cross-volume moves are refused initially.
If interrupted after retention, show recoverable missing destination, not success.
Rollback only returns the exact retained source to an absent destination; it never
overwrites subsequent user work. A link is removed as a link, never recursively.

**Corruption:** quarantine means marking an artifact unavailable for new attachment,
not moving/deleting the shared directory underneath live consumers. Report affected
attachments and artifact state. A replacement is a new fully verified generation;
retarget only explicitly selected consumers after qualification. Existing consumers
may still fail while referencing the corrupt generation; no availability claim.

**Recovery:** retained-source identity plus full manifest must match the intended
transaction/artifact evidence. Equal file counts are insufficient even for an
empty entry. Prepare a recovery plan; never guess the source or modify a live
shared target in place. Unknown provenance remains held for human investigation.

## 8. Concurrency and verification limits

Use kernel locks in a fixed cache-then-destination order. Locks coordinate nmpool,
not arbitrary installers/editors. Consumer processes must be stopped for adoption,
replacement and recovery; timestamps do not prove quiescence. Native Windows
handle-based identity and no-follow operations require a dedicated implementation
review and race rehearsal before any destructive operation is enabled.

Transactions use create-new records, atomic state publication and explicit durable
flush behavior where supported. Restart reconciles records and filesystem identity;
an interrupted or ambiguous state refuses further mutation. No power-loss guarantee
is claimed without tested filesystem durability semantics. Lost/malformed records
hold data, never authorize cleanup. No PID-age stale-lock override.

Full verification hashes every artifact at prepare/adopt, promotion and explicit
audit. Share attach performs bounded structural checks: valid published receipt,
root identity, directory existence, nonempty invariant when the manifest is nonempty,
and required profile sentinels with recorded identities/hashes. Missing Prisma
output or an emptied root must fail before publishing a link. Inspecting the same
artifact from multiple consumers can reuse one audit result for that operation.

Structural checks cannot detect arbitrary interior deletion or byte corruption.
Mtime, a sentinel and an old audit are not proof of unchanged bytes. Report this
limitation on every fast result; `--full` supplies a current full observation at
its measured cost. Known failures are sticky and block fast attach until a full
successful audit/qualified replacement, not merely until a timestamp changes.
Managed immutability does not make an audit linearizable against external writers.

## 9. Staged implementation plan

| Phase | Bounded work | Depends on | Gate |
|---|---|---|---|
| 0 | Review this design; capture sanitized actual install recipe, generator closure and commands; compare baseline and runtime writes | None | Agreement on managed-immutability risk and actual island boundary |
| 1 | Read-only island assessment plus small native Windows junction/identity/crash fixture, no live adoption | 0 | **Validation gate:** cache redirection works in two concurrent consumers; #6 race cannot move a target; otherwise stop root-junction design |
| 2 | Versioned policy and artifact receipts; npm v2/private-registry/local attestation; staged approved generation | 1 | Repeatable qualified artifact and Prisma/application checks; copy suite still green |
| 3 | Opt-in link into absent destinations; structural/full reports; exact consumer records | 2 | Native Windows failure suite and real-machine latency/isolation targets pass |
| 4 | Planned adopt candidate and promotion; retained provenance; replacement and explicit recovery | 3 | Crash matrix preserves both seed and rollback contents; independent exact-head review |
| 5 | Optional private facade, enforced protection or cleanup design | Evidence from 4 | Separate proposal; no automatic GC in this program |

Each phase is several small reviewed PRs, not one flags-and-refactor patch. Only
phases 0–1 are ready to detail before the validation gate. Sharing/adoption remain
disabled by default until their own gates pass. Update AGENTS.md's current-release
contract when implementation is admitted, not in this design-only change.

## 10. Open questions

- Is the actual consumer an independently installed npm island or a pnpm-managed
  closure? Obtain sanitized manifests/config and the real install command.
- Which exact formatting/build/test commands write inside `node_modules`, and
  which support redirection? #7's error is evidence to reproduce, not proof that
  precreating one directory solves the complete compatibility problem.
- Which generator inputs/env/native dependencies complete Prisma's artifact
  identity, and do generated files embed worktree-specific absolute paths?
- Can controlled generation replace adoption attestation for this workload?
- Does the operator accept managed immutability's shared blast radius, or must an
  enforced boundary/private facade qualify before real shared use?

## 11. Validation and issue acceptance

Native Windows tests must run on ordinary user permissions and exercise junctions,
broken junctions, case/short-path aliases where supported, junction ancestors,
held handles, destination creation races, source swaps, and interrupted transactions
at every recorded transition. Mutate lock/schema/runtime after planning and during
attach; require refusal, and reject an explicit artifact from another request key.
Compare full seed and retained manifests before and
after every attempted move, including refusals. Run native macOS copy regressions;
symlink sharing has a separate native qualification if enabled.

| Issue | Required evidence before closing |
|---|---|
| #6 | A consumer alias/junction can never cause pool-target movement; an empty/missing required tree refuses attach and flags every recorded affected consumer |
| #7 | Reported formatting command and other approved tools pass concurrently in two shared consumers with independent writable state and unchanged dependency manifest |
| #8 | Every retained tree has durable source/transaction/manifest provenance; equal-count decoy recovery is rejected; no automatic deletion path exists |
| #10 | Qualified actual Windows island supports the agreed share/adopt/script workflow while private copy remains supported; measured cost and assurance limits are published |

The Go incident's exact trigger is unconfirmed. Rehearsals establish Rust behavior,
not a retrospective root-cause claim. This document resolves neither the incidents
nor the implementation requirements; issue closure follows the evidence above.
