# Initial design and limits

The implementation is independent personal code. It does not contain or port the
unpublished Windows implementation described in an earlier handoff.

## Supported profile

`nmpool/npm-no-scripts/v1` accepts package-lock v3, public npm registry tarballs
with SHA-512 integrity, and no declared lifecycle scripts, workspace roots or
local/linked dependencies. The only permitted project npmrc setting is
`legacy-peer-deps`. Files are hashed as raw bytes; absence is distinct from an empty
file. Unknown configuration is refused without printing its values.

Preparation copies the manifest, lock and allowed npmrc into a private cache build
directory. It runs native Node against npm-cli.js with an empty environment except
OS-required variables and Node's directory on PATH, explicit empty user/global
configs and a staging-local npm download cache. Scripts, audit and funding are
disabled; dev/optional/peer dependencies are included. npm's version, distribution
tree hash, Node executable hash, runtime/architecture/OS identity and full fixed
recipe and effective file/directory creation permissions enter the key. No install commands run in a live source package. Tests/builds are
the consumer's responsibility; the receipt is not a claim of application correctness.

Roxiq's Sentry/esbuild/etc. install scripts are not yet supported. Ivy MCP is the
first real smoke workload. Script support requires reviewed complete inputs and a
fresh behavioral comparison, not a bypass flag. pnpm and external browser caches
are separate future work.

## Filesystem boundary

Each explicit cache root must be new/empty or carry our version marker. A kernel
file lock serializes cache mutation and inspection, without a lease expiration or
PID-based stale-lock override. Entry directories are keyed by a validated lowercase
64-character hex digest. Receipt contents are never used as filesystem paths.

An entry is visible only after its manifest and receipt have been written. Native
no-replace rename publishes on macOS and Windows. Failed preparation/restore staging
is retained for inspection. There is no garbage collector or adoption operation.
Publication is atomic against destination replacement, but not a power-loss durability
guarantee; after restart, receipt and full content verification are required again.

Every restore scans/hashes the source, copies to private staging, scans/hashes the
copy, rechecks package and toolchain inputs, and publishes only to an absent
node_modules. A destination lock coordinates nmpool callers. Regular-file identity
must differ between source and destination. Unix relative links are accepted only
when resolving to regular files inside the tree. Windows reparse points, including
junctions, are refused by the initial restore profile; native npm executable shims
are ordinary files and are supported. Unknown names/types are refused.

On macOS native file copying may clone or copy; the tool deliberately reports clone
use and physical savings as unmeasured. On Windows the initial adapter copies the
verified primary stream into a create-new file, not alternate streams. It preserves
readonly permissions but does not promise arbitrary Windows metadata/ACL equivalence.
ReFS acceleration is not implemented. No hardlinks to cache data, filesystem migration,
administrator requirement, daemon or Fleet integration is involved.

This protects against ordinary accidental changes, concurrent nmpool operations and
cache corruption. It is not a sandbox against a malicious process running as the same
user. The user must exclusively own the destination while restoring; unrelated npm,
watchers or editors do not honor nmpool's lock. The code never moves or deletes an
existing dependency tree and never assumes mtime proves idleness. A consumer may
mutate its private install after restore, but it is never recaptured implicitly.

## Census

The census runs only Git enumeration and filesystem reads. It deduplicates physical
worktrees/package/install identities and preserves repeated-enumeration counts.
Package traversal is bounded and does not follow directory links. Its JSON separates
candidate input groups from a reuse key (always null during census). Partial scans
exit 2. Unsupported install profiles are reported per row; missing worktree/read
errors are never interpreted as zero references or permission to delete.

## Restoration provenance and drift

Restore writes a create-new `.nmpool-restore.json` inside the private staged tree,
then publishes tree and record in one no-replace rename. It refuses a collision
with a package-owned file. The record embeds the verified cache receipt plus a
Unix timestamp and best-effort Git branch/commit context, which never enter the
cache key. It is not a signed or tamper-proof activity history.

Status takes a shared existing destination lock, validates the receipt's schema,
input/runtime key and artifact fingerprint, and compares the full installed tree
(excluding only that root receipt file) with the original manifest. It names
added/removed/modified paths and separately compares current input/runtime fields.
It rechecks inputs/toolchain and record contents before reporting. External editors
and installers do not honor the lock: this remains an observation under exclusive
user ownership, not a linearizable filesystem snapshot. No cache is needed.
Missing receipts are untracked; invalid receipts, unsupported filesystem reads and
unavailable toolchains cannot yield clean. Unsupported current package inputs
produce a non-clean report. The command never rewrites an install or its record.

Explain compares supported package inputs under selected toolchains, emitting keys
and differing field names. It does not decode dependency-version changes, infer
changes from a branch name, or attribute them to tasks/processes. Both status and
explain execute identity probes (including temporary permission probes); neither
runs an npm installation. Old installs are not retroactively recorded.

## Validation and release boundary

Native macOS and Windows contract tests must pass from the first PR. Tests exercise
private mutation, corruption, atomic no-overwrite publication, competing cache locks,
key changes, incomplete receipts, existing destinations and real Git worktrees.
Platform tests cover Unix links and Windows junctions/held handles. A fresh npm
prepare/restore/inspect CLI test runs without network dependencies on both platforms.

The first personal workload uses an isolated Ivy worktree; live installs stay
untouched. Timing must include toolchain fingerprinting and all integrity checks.
Measure cold seed cost and repeated restore against warm npm ci separately. Native
Windows CI establishes behavior on its runner, not benefit on the actual work laptop.
No disk savings, work-laptop acceptance, independent review or merge is implied by
local test success. Do not broaden scope on the basis of one small-package benchmark.
