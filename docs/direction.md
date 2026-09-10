# Direction: copy mode is correct and not enough

**Status:** measured conclusion and proposed shape; nothing here is implemented.
The merged tool is copy mode only.

## What was measured

The tool was written against small, clean packages, where every operation is fast
and the interesting property is provenance. Its intended consumer is different: one
npm island inside a large private monorepo, with many git worktrees, most of which
have no install at all because a cold `npm ci` costs about ten minutes each.
Measured on one such island (figures rounded; the repository is not identified):

| Property | Value |
|---|---:|
| Installed tree | about 2 GiB |
| Files / directories | about 110k / 14k |
| Locked packages | about 2.3k |
| Git worktrees of the repository | about 130 |
| Worktrees with a real install | about 20 |
| Worktrees with a link to a shared install | about 10 |
| **Worktrees with no install at all** | **about 100** |
| Cold `npm ci` | about 10 minutes |

The empty worktrees are the entire prize. Disk is a distant second concern.

| Operation on that tree | Time |
|---|---:|
| Create a junction to an existing tree | about 0.1 s |
| Metadata walk, no hashing, single thread | about 26 s |
| Native parallel copy, no verification | about 48 s |
| sha256 of every file, single thread | about 250 s (IOPS bound) |

Two conclusions follow.

- **Sharing beats copying by roughly 500× before verification.** A design whose
  materialization step is a copy cannot serve this consumer, whatever the profile
  gates allow.
- **Full-artifact verification on every operation does not fit at this scale.** The
  current restore hashes the source, copies, then hashes the copy. Even with a
  hasher as fast as the parallel copy that is two and a half minutes to avoid a
  ten-minute install. The integrity story that is free on a 50 MB tree is the
  dominant cost on a 2 GiB one.

The same island fails the current profile on five separate gates at once:
lockfile v2, packages declaring install scripts (an ORM client with a native
engine, a bundler, an image library, a headless browser, a native tracer), several
hundred dependencies resolved from a private registry, several hundred packages
carrying no `integrity` field, and an `.npmrc` setting outside the permitted key.
Refusing is correct today; it also means the current profile is four separate scope
expansions away from its consumer.

## The correctness trap behind script support

Some install scripts write generated content into `node_modules` from inputs that
are not npm inputs. The measured island's ORM generates a typed client plus a native
query engine from its schema file. Keying on the lockfile alone yielded 6 distinct
installs; keying on the lockfile *and* the schema yielded 15. Nine groups would be
silently merged, handing one worktree a client generated from another's schema.
That is the one failure that corrupts instead of refusing. `--ignore-scripts`
currently hides it by never generating the content, which is why the profile
refusal looks safe; the moment scripts run, the key is wrong unless generator
inputs enter it. They must be declared per island, never sniffed.

## Proposed shape: one key, one cache, two materialization modes

The two problems differ on exactly one axis: whether the destination may share a
writable tree with its siblings. The key, cache layout, receipt and drift reporting
are common, so materialization becomes a strategy rather than a fork.

```text
key   = H(profile_id, manifest, lock, allowed npmrc, runtime identity,
          recipe, permissions, generator_inputs[])
entry = cache/<key>/{ manifest.json, receipt.json, tree/ }

materialize(entry, dest, mode):
    mode == copy  -> verified private copy        (today's behavior)
    mode == share -> junction | symlink to entry  (new)
```

- **copy** stays exactly as built and stays the default for small clean-profile
  packages.
- **share** publishes the entry once, then points `dest/node_modules` at
  `cache/<key>/tree`. It still refuses a non-absent destination. The receipt lives
  in the entry, with a small per-worktree link record naming the key. Refcount by
  scanning link targets, never by a counter, because counters go stale when git
  removes a worktree underneath the tool.

Four things the design needs that the current tool refuses:

1. **Generator inputs in the key**, declared per island. A correctness requirement,
   not a feature.
2. **Adoption, gated and explicit.** A machine with existing real installs must be
   able to bootstrap: verify an existing tree against the key, hash it once, move
   it into the cache, link the source back. It mutates, so it wants a dry run and
   per-island opt-in.
3. **Tiered verification.** Pay in full once, at prepare or adopt. Afterwards
   `share` verifies the entry against its stored manifest digest and that the
   destination is absent, without re-walking the tree. `status` goes cheap by
   default with `--deep` for the full hash, keeping exit 2 for drift.
4. **Profile gates as facts, not attacks.** What the design needs is that inputs
   are enumerated and hashed as bytes and the install is reproduced by a recipe the
   tool controls. It does not need the registry to be npmjs.org. For
   integrity-less tarballs from a private registry, record the resolved URL plus a
   digest computed at prepare time so the entry is self-verifying. Registry
   credentials come from the environment, never from a captured npmrc.

## What this means for the merged tool

The copy-mode implementation, its safety contract (never adopt, link, replace or
delete an existing install) and its tests stay. Share mode is the next design, and
it should be a reviewed document before it is code. Rust stays.
