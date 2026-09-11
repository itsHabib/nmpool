# Command reference

| Command | What it does | Changes |
|---|---|---|
| `scan` (alias of `census`) | Inventory Git worktrees and identify supported input candidates | None; no npm execution |
| `prepare` | Build a cache entry with fresh npm ci, or verify an existing matching entry | Cache only; not the source install |
| `restore` | Verify and copy an entry into an absent node_modules | New private install plus a destination lock |
| `inspect` | Read the receipt and verify every cached artifact | None |
| `status` | Compare this restored install with its receipt and current inputs/runtime | No project/cache writes |
| `explain` | Compare two packages' requested installs and name differing inputs | No project/cache writes |

`prepare` does not capture an existing install. `restore` does not fetch packages
or fall back to npm on a miss. `inspect` verifies the cache, not application tests.
There is no automatic integration with Git worktree creation.

On Windows, use absolute paths (`C:\repo\package`) or ordinary relative paths
(`.\package`). Drive-relative forms such as `C:package` are refused because their
meaning depends on hidden per-drive working-directory state.

## Daily use

```sh
# Inventory first; unsupported_reason explains packages outside this profile.
nmpool scan --repo /path/to/repo --json
# See whether two worktrees need the same installation.
nmpool explain --package /path/to/first/package --against /path/to/second/package
# Prepare once, then restore only into a worktree without node_modules.
nmpool prepare --package /path/to/first/package --cache /path/to/private-cache
nmpool restore --package /path/to/second/package --cache /path/to/private-cache
# Check before running the package's own tests/build.
nmpool status --package /path/to/second/package
```

`status` prints JSON. Exit 0 means inputs/runtime and installed files match the
restoration snapshot; exit 2 means `drifted`, `untracked`, or `absent`; exit 1 means
verification failed. Drift lists added/removed/modified files separately from
input differences. Unsupported current inputs appear in `input_error`. No receipt
means untracked, including installs from older nmpool builds; no adoption occurs.

`explain` names differing input fields (for example `/inputs/files/package-lock.json`)
and emits both keys. It compares file fingerprints after JSON CRLF normalization, not individual
dependency version changes. `input_file_details` explains line-ending-only changes
even when keys match, and equivalent JSON representation changes when keys
still differ. Other whitespace is intentionally significant. Branch names are context: different branches can request the same
install. By default it compares both packages under the selected current Node/npm;
use `--against-node` and `--against-npm-cli` to select a different second runtime.
A successful comparison exits 0 even when requirements differ.

Both commands accept `--node` and `--npm-cli`. They execute local Node/npm identity
probes and Git reads, but no install commands; probes create temporary files outside
the project. Stop concurrent installers/editors for a consistent snapshot.

Every new restore includes `node_modules/.nmpool-restore.json`: the original input
and artifact receipt, restoration time, and available Git branch/commit context.
The record is published atomically with the install, works without the cache, and
moves with the worktree. It is local provenance, not a tamper-proof audit or a record
of which process/agent changed files. Do not edit the receipt. A reserved-name
collision is refused. Running npm ci may remove it, making the install untracked.

`status` never repairs, deletes, or refreshes an install. If inputs changed, prepare
the new requirement and use a fresh worktree. A clean status does not replace your
application tests or establish continuous monitoring.

## Use

```sh
nmpool scan --repo /path/to/repo --json
nmpool prepare --package /path/to/package --cache /path/to/private-cache
nmpool restore --package /path/to/new-worktree/package --cache /path/to/private-cache
nmpool inspect --cache /path/to/private-cache --key KEY_FROM_PREPARE
```

Use native Windows paths for the same arguments in PowerShell. Node is discovered
on PATH; `--node` selects an executable. npm's standard layout is detected, or supply
`--npm-cli C:\path\to\node_modules\npm\bin\npm-cli.js` explicitly. npm runs through
Node, avoiding shell parsing of an npm.cmd command line.

`census` is read-only. Repeat `--repo` to scan more than one repository; registered
worktrees are deduplicated. Default package depth is two; `--max-depth` permits up to
eight. JSON includes incomplete-scan errors, candidate input groups and link states.
Exit 0 means complete within scope, 2 partial census, 1 failure.

`prepare` creates a fresh install in private staging, never in the source package.
`restore` only accepts an absent node_modules. It leaves a `.nmpool.lock` in the
destination package to coordinate callers; add it to your local ignore rules if
needed. Do not run other installers/watchers in that destination during restore.
`inspect` verifies the full cache entry without creating files.

## Deliberately narrow first release

- npm package-lock v3 and integrity-pinned public npm registry dependencies.
- No lifecycle scripts, workspaces, local/Git dependencies or arbitrary npmrc.
- All installs use `npm ci --ignore-scripts` with controlled configuration.
- Full artifact verification on every restore; no trust based on lock equality alone.
- Native private copying, with clone use and physical storage savings **unmeasured**.
- No shared writable node_modules, adoption, GC, remote cache or plugin framework.

Roxiq's install scripts are currently refused; Ivy MCP is the initial supported
personal workload. This is a usable experimental mechanism, not a performance claim.
Preparation staging, npm download caches, and logs are retained under `CACHE/staging`
on success or failure; failed npm runs retain stdout and stderr in `install.log`.
Failed restore staging is retained under the destination package. These are for inspection;
they can consume disk space. There is no automatic cleanup. Never point the cache at
an existing dependency tree or an unrelated nonempty directory.

See [design and threat boundary](design.md) and [validation](validation.md).

## Sharing assessment and protection rehearsal

See [sharing qualification](sharing-trial.md) for native Windows commands and the
work-agent handoff. `assess --package PATH` always exits 2 after a complete report:
sharing remains unqualified. `protection-probe --parent PATH` creates a disposable
fixture on the selected volume and exits 0 only if its permission checks pass,
2 for failed checks, or 1 for a setup/read error. Both emit JSON. Neither changes
an existing install or enables sharing.
