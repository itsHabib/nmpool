# pnpm workspace installs — Technical Design Document

**Status:** proposed. Phase 0 is a measurement that may end this work with no code.
**Owner:** @itsHabib · **Date:** 2026-09-17
**Related:** [#23](https://github.com/itsHabib/nmpool/issues/23),
[shared installs](../share-adopt/spec.md), [current contract](../../design.md).

> Review focus: whether nmpool should build anything here at all; the split between a
> shared store and private importer link farms; what is still unknown on Windows.

## 1. Problem

Issue #23: only npm islands are pooled. A consumer's pnpm workspace still pays a full
`pnpm install` per worktree. The reporter measured 3 to 9 minutes and 1 to 3 GB each, and
seven parallel installs filled the disk on 2026-09-16. The reference consumer is a
pnpm 10.13 workspace: lockfile v9, about 3,700 locked packages, five importers, five
`workspace:` dependencies, an allowlist of dependencies with install scripts, and no
schema-style generator inside the workspace.

The issue proposes keying on the pnpm lockfile and linking both the root `node_modules`
and a workspace package's `node_modules` to a shared tree, noting that a hand-made version
of this "works". Section 2 shows that the second half of that proposal is silently wrong.

## 2. Findings (reproduced, macOS, pnpm 10.13.1)

The script in the appendix builds a two-package workspace (`app` depends on `@x/ui` by
`workspace:*` and on two registry packages) and probes three sharing arrangements from a
second worktree whose `@x/ui` source differs.

**Layout.** The root `node_modules` holds the virtual store (`.pnpm`), `.modules.yaml`
and workspace state: all of the bytes. An importer's `node_modules` is a small farm:
relative links into the root store for registry dependencies, a relative link to the
sibling's **source directory** for each `workspace:` dependency, and `.bin` shims.

| Arrangement | Workspace package resolved | Registry deps | Verdict |
|---|---|---|---|
| A. Root linked to a shared tree; importer farms private real directories | the second worktree's own source | shared tree | correct |
| B. Root **and** importer `node_modules` linked to the first worktree's | the **first** worktree's source, no error | shared tree | silently wrong |
| C. pnpm `enableGlobalVirtualStore: true`, ordinary frozen offline install | own source | global store | correct; `node_modules` was about 20 KB |

B fails because link resolution is physical: a relative `workspace:` link inside a shared
importer directory resolves inside the tree that owns the directory. Every consumer of
that shared directory imports one worktree's workspace sources. Nothing reports it.

C is pnpm's own answer to this problem (10.12+, experimental): importer farms link
straight into `<store>/links/<name>/<version>/<graph hash>`, so a worktree's install is
link creation only.

## 3. Decision ladder

Build only what the outcome requires. The outcome is: a new worktree of a pnpm workspace
gets correct dependencies in seconds with near-zero additional disk.

1. **Phase 0 — measure pnpm's global virtual store on the consumer. No nmpool code.**
   If it meets the acceptance bar in section 9, nmpool documents the setting, teaches
   `census` to recognise pnpm installs, and closes #23. A second mechanism that duplicates
   a package manager's own feature is maintenance with no benefit.
2. **Phase 1 — a pnpm island (arrangement A), only if phase 0 fails** on a named,
   measured blocker: tooling that breaks when resolved paths leave the project, a Windows
   limitation, or built dependencies that misbehave from the global store.

Arrangement B is rejected in every phase, including adoption of an existing tree.

## 4. Requirements for phase 1

- One shared, write-protected generation holds the root `node_modules` of a workspace.
- Every importer's `node_modules` is a private real directory in the consumer worktree,
  rebuilt from a recorded farm manifest. No importer directory is ever a link.
- `workspace:` links always resolve to the consumer worktree's own packages.
- The generation is location independent: no link inside it may resolve outside it.
  Publication refuses a tree that violates this.
- Same refusal posture as npm sharing: explicit profile, explicit artifact, absent
  destinations only, no install on miss, no GC, recovery by recorded transaction.
- Private copy mode and npm sharing are unchanged.

Non-goals: Yarn, Bun, `node-linker=hoisted` or `pnp`, injected workspace packages, and
running pnpm inside a consumer worktree on the user's behalf.

## 5. Architecture (phase 1)

`prepare --profile` stages the workspace manifests the lockfile names (root and every
importer `package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`, permitted `.npmrc`,
declared generator and context inputs), with importer source trees absent, and runs the
profile's approved `pnpm install --frozen-lockfile` with an isolated store and
configuration. It then:

1. Records each importer's farm as data: entry name, kind (`store-link` with a target
   relative to the root `node_modules`, `workspace-link` with a workspace-relative path,
   or `shim` with bytes), in a `farms.json` bound to the generation header.
2. Verifies that every link in the root tree resolves inside the root tree.
3. Publishes the root tree as the generation, protected like an npm generation.

`link` attaches the root `node_modules` exactly as today, then materialises each
importer's farm privately from `farms.json` using native links. It requires every
importer `node_modules` to be absent and records all of them in one attachment
transaction, so `unlink` and `recover` remove exactly what was created.

`prepare --base` (merged in #36) already exports declared inputs at a revision; the
importer manifests join the exported set, so pre-warming works unchanged.

## 6. Key and profile

The request key covers the lockfile, `pnpm-workspace.yaml`, every importer
`package.json` (subject to `runtime_only_scripts`), permitted pnpm configuration, the
pnpm version and distribution hash alongside Node's, the policy hash, and generator
inputs. `program` gains `pnpm`, resolved like npm today: through Node against a named
CLI entry point, never through a shell shim. `independent_npm_island` is not asserted
for a pnpm profile; the workspace root is the island.

## 7. Failure flows

- An importer `node_modules` already exists: refuse the whole link, create nothing.
- A farm entry cannot be created: roll back the farms created in this transaction,
  then the root attachment; leave a recoverable transaction if rollback fails.
- The consumer's workspace layout differs from the generation's importer list: the
  request key already differs, so `link` refuses with `request_mismatch`.
- A tool writes into an importer farm: it is private, so only that worktree is
  affected; `shared-status` reports the farm as changed against `farms.json`.
- pnpm is run inside an attached worktree: it fails against the protected root.
  Documented, not prevented.

## 8. Unknowns that phase 0 and a Windows trial must answer

1. pnpm creates directory links as junctions on Windows, and junction targets are
   absolute. A virtual store built in staging may therefore not survive the move into
   the pool. Options are building at the final path or rewriting verified in-tree
   junctions at publication. Neither is chosen until measured on Windows.
2. Whether the consumer's build and test tooling tolerates resolved paths outside the
   project root (global store) or inside a pool (phase 1). Bundlers and test runners
   with filesystem allowlists are the usual failures.
3. Whether dependencies with install scripts behave when their built output lives in
   a shared location.
4. Farm creation cost on Windows for the real importer sizes.

## 9. Validation

Phase 0 acceptance, measured on the real consumer on Windows, frozen offline install in
a fresh worktree with the global virtual store on versus off:

- install wall time under 30 seconds with it on;
- added disk per worktree under 50 MB;
- the consumer's own build and test commands pass from that worktree;
- the appendix probe prints the worktree's own workspace source.

Phase 1 acceptance adds: two consumers of one generation each resolve their own
workspace sources; an importer directory is never a link; a tree with an escaping link
is refused at publication; `unlink` removes the root attachment and every farm it
created and nothing else; native macOS and Windows CI.

## 10. Open questions

- If phase 0 passes, is a `census` row that recognises pnpm installs worth adding, or is
  documentation enough?
- Should adoption of an existing pnpm tree exist at all, given arrangement B is the
  shape existing hand-linked trees are likely to have?

## Appendix: reproduction

```sh
mkdir -p ws/app ws/packages/ui && cd ws
printf 'packages:\n  - app\n  - packages/*\n' > pnpm-workspace.yaml
echo '{"name":"root","private":true}' > package.json
echo '{"name":"app","version":"1.0.0","dependencies":{"@x/ui":"workspace:*","is-number":"7.0.0","semver":"7.6.3"}}' > app/package.json
echo '{"name":"@x/ui","version":"1.0.0","main":"index.js"}' > packages/ui/package.json
echo 'module.exports="ui-A"' > packages/ui/index.js
pnpm install && cd ..

# A second worktree whose workspace source differs.
copy() { mkdir -p $1/app $1/packages/ui; cp ws/package.json ws/pnpm-workspace.yaml ws/pnpm-lock.yaml $1/
  cp ws/app/package.json $1/app/; cp ws/packages/ui/package.json $1/packages/ui/
  echo "module.exports=\"ui-$2\"" > $1/packages/ui/index.js; }

copy a B; ln -s "$PWD/ws/node_modules" a/node_modules; cp -R ws/app/node_modules a/app/node_modules
(cd a/app && node -e 'console.log(require("@x/ui"))')   # ui-B: correct

copy b C; ln -s "$PWD/ws/node_modules" b/node_modules; ln -s "$PWD/ws/app/node_modules" b/app/node_modules
(cd b/app && node -e 'console.log(require("@x/ui"))')   # ui-A: the other worktree's source

copy c D; printf '\nenableGlobalVirtualStore: true\n' >> c/pnpm-workspace.yaml
(cd c && pnpm install --frozen-lockfile --offline && du -sh node_modules && cd app && node -e 'console.log(require("@x/ui"))')   # ui-D
```
