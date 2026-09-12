# Shared installs and adoption

Experimental implementation on the live-sharing branch. Native validation and
review must pass before release. Private `prepare`/`restore` behavior is unchanged
unless an explicit `--profile` is supplied.

## Declare the package

Copy `examples/island.json` and edit it for your actual npm package. Its commands
are examples, not discovery: `generate` and `test:dependencies` must exist and the
application must actually use the configured private cache path. Include every
schema, imported generator source and configuration file under `generator_inputs`.
List workspace/config files inside this checkout, such as `../pnpm-workspace.yaml`,
under `context_inputs`; this records context without installing the parent workspace.
Automatic discovery stops at the nearest `.git` directory or worktree `.git` file,
including that directory. It does not inherit a home `.npmrc` or an outer checkout.
For an unversioned package only its own directory is checked automatically; explicitly
list any relevant parent context. Declared context is always captured, even outside
the automatic boundary. Missing-context errors list every required relative path.
Staged builds still use empty user/global npm config; this does not import registry
credentials from your interactive npm setup.

The profile supports npm lockfile v2/v3, lifecycle scripts and HTTPS private
registries. Missing upstream integrity needs `allow_local_attestation`; that
records observed bytes, not publisher authenticity. List every permitted HTTPS
registry host (including any explicit port) in `registry_hosts`; both lockfile
URLs and npmrc registries must match. `trust_domain` separately identifies the
nonsecret authorization context. Each generation stores a protected, hash-bound
`provenance.json` with observation time, source hosts and supplied upstream
integrity. This detailed record is checked during full audits. An independent npm island is
an explicit reviewed assertion, not something a pnpm parent proves. Linked/local
workspace dependencies remain unsupported by this root-link strategy.

Command `program` accepts only `node` or `npm`. To run a locally installed tool,
use an npm script (for example, `"program": "npm", "args": ["run", "check"]`),
not `"program": "npx"`. Profile parse failures retain the parser's diagnostic and location.

Commands run against staging with a restricted environment and private npm cache.
They are **not sandboxed**. Do not approve migrations or commands that mutate live
sources. Put nonsecret environment names in `selected_env`. Credential names go
in `credential_env` and npmrc uses placeholders such as `${NPM_TOKEN}`; never put
literal credentials in a profile. Commands' output is suppressed to avoid leaking
credentials. Check required inputs and commands before running the profile.

## Reuse across task-script changes

By default the complete `package.json` remains an input: arbitrary install or generator
code can read it. Do not simply discard all fields outside dependency declarations.
For task shortcuts that must not participate in an install, the optional profile field
`"runtime_only_scripts": ["dev", "lint:local"]` removes only those named scripts from
**both the staged package.json and its input key**. Use the same profile in all consumers.
All other fields and scripts remain keyed; known install lifecycle hooks cannot be
excluded. Build and validation commands must not depend on excluded scripts. Their
removal is real, not an assertion that a hidden input is harmless.

The live consumer's package.json is never rewritten. Runtime commands still use it.
Source bytes are checked for concurrent changes, and source provenance v2 records the
original package.json digest separately. Adoption remains local attestation: verify the
exact candidate against the normalized recipe before trusting it. This option does not
prove a pre-existing install was built from those inputs.

The sharing input recipe is now `nmpool/island-inputs/v2`; existing request keys no longer
match. Keep old generations, original installs and recovery records. Prepare a new
generation (or explicitly adopt and qualify a candidate) for new attachments; do not
edit old receipts or remove a live attachment to force compatibility. Private restore
keys are unchanged.

## Prepare once and link

```sh
nmpool prepare --package /repo/web --cache /pool --profile /repo/island.json
# Copy artifact_id from the JSON output.
nmpool link --package /other-worktree/web --cache /pool --profile /repo/island.json --artifact ARTIFACT_ID
nmpool run --package /other-worktree/web --cache /pool --profile /repo/island.json --tool check
nmpool shared-status --package /other-worktree/web --cache /pool --profile /repo/island.json
nmpool shared-inspect --cache /pool --artifact ARTIFACT_ID --full
```

`link` creates a junction on Windows or symlink on macOS. It requires matching
policy, input and runtime identity and an absent destination. Repeating a matching
attachment validates it before returning. Dependencies are protected against
ordinary writes through the dependency tree, including its immediate guard. This
is not a sandbox: programs running as the pool owner can edit writable store
ancestors or deliberately change permissions. Do not grant untrusted code access
to that account or pool. Moving the pool makes attachments invalid; it does not
redirect or silently repair them. Each runtime tool
gets a separate `.nmpool-runtime/<attachment>/<tool>` directory. `{runtime}` in
approved arguments or environment values expands to that directory. A tool that
cannot redirect writes outside `node_modules` is incompatible with this strategy.

Pool-changing operations are serialized. `run` waits for the current pool operation
(including a long prepare or full audit) at startup and its final check; cancel it
with Ctrl-C if you do not want to wait. The pool lock is released while the tool runs.

Fast results check bounded metadata, physical identity and required file probes.
They are not fresh full-content verification. `shared-status` exits 2 for this
non-clean observation; explicit `shared-inspect --full` hashes the whole tree.
Detected content/protection failures quarantine a generation for future access;
prepare a new generation instead of editing published bytes.

## Adopt, qualify, then replace explicitly

Stop installers, application processes and watchers for this package first.

```sh
nmpool adopt --package /repo/web --cache /pool --profile /repo/island.json --plan
nmpool adopt --package /repo/web --cache /pool --profile /repo/island.json --plan-id ADOPTION_PLAN
nmpool qualify --package /repo/web --cache /pool --profile /repo/island.json --artifact CANDIDATE_ID
nmpool link --package /repo/web --cache /pool --profile /repo/island.json --artifact CANDIDATE_ID --plan
nmpool link --package /repo/web --cache /pool --profile /repo/island.json --plan-id REPLACEMENT_PLAN
```

Adoption copies and fully verifies a candidate, leaving the original untouched.
Repeating the same adoption plan resumes its recorded candidate after rechecking
the source, candidate and transaction; it does not silently select different bytes.
Qualification runs the policy's checks on a separate copy and binds their success
to the exact candidate content. This remains local attestation; it does not claim
the lockfile produced an existing installation.

Replacement rechecks the plan and preserves the original at
`shared-v1/retained/<transaction>/tree`, with provenance beside it. The move must
remain on the same volume. A failure keeps data for explicit recovery. Unknown
links and pool aliases are never treated as ordinary trees to move.

```sh
nmpool retained --cache /pool
nmpool recover --cache /pool --transaction REPLACEMENT_PLAN --plan
nmpool recover --cache /pool --transaction REPLACEMENT_PLAN --execute
```

Recovery checks the original physical identity and full manifest. For a first-time
attachment, the displayed `remove-attachment` operation removes only that exact
link and leaves the package without `node_modules`; it does not install a replacement. It only removes
the transaction's own link and never overwrites a new ordinary install. There is
no garbage collector. Retained records and failed build staging are deliberately kept;
successful build and qualification copies are removed. If cleanup fails after a
successful publication or qualification, the command still returns its successful
result with `cleanup_warning`; inspect the named staging directory separately. An interrupted local link
creation is recorded before the link exists: `recover` reports `remove-staging-link`
when publication never reached its prepared record. It validates the private staging
directory and intended target before removing only the link. Changed or unavailable
targets in that early window are held for inspection. During replacement rollback,
`staging_held: true` reports that leftover separately: it never prevents restoring
the independently verified retained original.
Power-loss durability and the actual work-laptop application/performance trial
remain separate from local fixture validation.
