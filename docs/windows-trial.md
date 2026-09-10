# Try nmpool on a Windows machine

This is a standalone, experimental tool. Start with a disposable worktree of one
package. Native Windows CI passes, and the maintainer has reported a successful
work-agent run. Use this guide to check your package and record its results.
The small Mac/Ivy trial was slower than warm npm ci.

## Get the implementation

Use the merged implementation on `main`. In a directory where a new `nmpool`
checkout can be created, use PowerShell:

```powershell
git clone https://github.com/itsHabib/nmpool.git nmpool
if ($LASTEXITCODE -ne 0) { throw 'Clone failed' }
Set-Location nmpool
git rev-parse HEAD  # Record this revision with your results.
cargo install --path . --locked
if ($LASTEXITCODE -ne 0) { throw 'Build/install failed' }
$nmpool = Join-Path $HOME '.cargo\bin\nmpool.exe'
& $nmpool --version
if ($LASTEXITCODE -ne 0) { throw 'nmpool did not start' }
```

Source installation needs Rust 1.89+ with native Windows build tools. No WSL is
needed. Native Node/npm are needed for prepare/restore; Git is needed for census.
Python 3 plus Rust's rustfmt/clippy components are needed only if you also run
`python scripts/check.py` yourself.

Alternatively, download the `nmpool-Windows-X64` artifact from a successful
[CI run for main](https://github.com/itsHabib/nmpool/actions/workflows/ci.yml?query=branch%3Amain), extract it, and set
`$nmpool` to that executable's full path. This avoids installing Rust. Record the
run's commit; artifacts expire after 14 days and are not a signed release or an
installer. Downloading Actions artifacts requires GitHub sign-in. The Windows artifact targets x64; native Windows ARM64 is untested.

## Quick smoke test before choosing a work package

From the nmpool source checkout, with Python 3 and native Node/npm installed:

```powershell
python scripts/smoke.py --binary $nmpool
if ($LASTEXITCODE -ne 0) { throw 'Smoke test failed; see printed evidence path' }
```

This uses an empty dependency fixture in a new temporary directory: no downloads,
project repository, credentials or live install are needed. It checks prepare, restore,
clean status, refusal to overwrite, consumer drift and unchanged cache integrity.
Both native CI runners execute the same script. It retains `results.json` and the
fixture at the printed path on success or failure; delete only that disposable
trial directory when you are finished. This is a mechanics test, not a performance
benchmark or evidence that a private-registry monorepo is supported.

For the trial, use an ordinary local directory. OneDrive placeholders and other
reparse points are refused with `link_or_reparse_path`; hydration/sync status does
not override that boundary. Do not disable the guard to accept a synced path.

## First check whether the package is supported

```powershell
$repo = 'C:\src\your-repo'  # Change this to the Git repository root.
& $nmpool scan --repo $repo --max-depth 4 --json
# Exit 0: complete within the requested depth. Exit 2: partial; inspect errors.
# Exit 1: command failed. This command changes no files and runs no npm.
```

Look for the package's row and `unsupported_reason`. A null reason means its
inputs are candidates, not that its application has been tested. `reuse_key` and
physical storage savings remain null during a scan. `scan` is an alias of `census`.

The initial profile requires npm package-lock v3 and public-registry packages.
Stop if this workload needs any of the following:

- An npm/pnpm workspace, local/Git dependencies, or another package manager.
- Root or dependency installation scripts (including packages flagged with
  `hasInstallScript` in the lockfile).
- Private registries, authentication, proxy/custom CA settings, or arbitrary npmrc.
  Only project `legacy-peer-deps=true/false` is supported. Preparation deliberately
  does not inherit your normal npm credentials, proxy or certificate environment.

This restriction is likely to exclude some private monorepos. Record the refusal;
do not edit the lockfile or remove scripts to make a production workload pass.

## Prove one restore works

Edit `$subdir` to the package directory relative to the Git root (`.` for root).
All following work happens in newly created worktrees at the same commit. Any
uncommitted changes in your original checkout are intentionally excluded.

```powershell
$subdir = '.'
$trial = Join-Path $env:TEMP ('nmpool-trial-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $trial -ErrorAction Stop | Out-Null
$baseline = Join-Path $trial 'baseline'
$restored = Join-Path $trial 'restored'
$cache = Join-Path $trial 'cache'
$revision = git -C $repo rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve source revision' }
git -C $repo worktree add --detach $baseline $revision
if ($LASTEXITCODE -ne 0) { throw 'Cannot create baseline worktree' }
git -C $repo worktree add --detach $restored $revision
if ($LASTEXITCODE -ne 0) { throw 'Cannot create restore worktree' }
$baselinePackage = Join-Path $baseline $subdir
$restoredPackage = Join-Path $restored $subdir

$seedJson = & $nmpool prepare --package $baselinePackage --cache $cache
if ($LASTEXITCODE -ne 0) { throw 'Prepare failed; inspect the reported error/log' }
$seed = $seedJson | ConvertFrom-Json
$seedJson | Set-Content (Join-Path $trial 'prepare.json')
$restoreJson = & $nmpool restore --package $restoredPackage --cache $cache
if ($LASTEXITCODE -ne 0) { throw 'Restore failed' }
$restoreJson | Set-Content (Join-Path $trial 'restore.json')
& $nmpool status --package $restoredPackage
if ($LASTEXITCODE -ne 0) { throw 'Fresh restore does not match its receipt' }
& $nmpool explain --package $baselinePackage --against $restoredPackage
if ($LASTEXITCODE -ne 0) { throw 'Input comparison failed' }
# same_install_requirements must be true for these identical package snapshots.

& $nmpool inspect --cache $cache --key $seed.key | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Cache verification failed' }
& $nmpool restore --package $restoredPackage --cache $cache
if ($LASTEXITCODE -ne 1) { throw 'Expected refusal of an existing destination' }
# The second restore must report destination_exists, not another error.
```

Now run the package's **real test/build command** in `$restoredPackage`. A successful
restore is not a passing application test. To establish the baseline, run
`npm ci --ignore-scripts` and the same tests in `$baselinePackage`, which prepare
left without node_modules. Stop if baseline tests also fail: the scripts-disabled
profile may not be useful for this package.

For the independence check, edit a disposable ordinary file under the restored
node_modules, then run `status --package $restoredPackage`: it must exit 2 and
name that file under `file_changes`. Repeat `inspect`; cache verification must
still succeed. Keep
this mutation separate from the application test result. Do not edit cache files.
The cache is local to this machine/runtime; do not copy a Mac cache to Windows.

## Check as work evolves

Run `status --package PATH` before relying on a restored install. It separates
changed inputs/runtime from changed installed files; a clean snapshot exits 0.
Exit 2 also covers absent or untracked installations. Corrupt receipts and read
errors exit 1. No nonzero result means safe to ignore or automatically replace.

Use `explain --package PATH --against OTHER_PATH` to compare worktree requirements.
It shows differing input fields and both cache keys, with Git context. It does not
identify which task/agent/process made the change. Existing installs from an older
build have no restoration record and remain untracked; use a fresh trial worktree.
These commands do not install packages, but invoke Node/npm identity probes.

## Decide whether it helps

For a comparable timing trial, use the same Node/npm versions, package revision,
legacy-peer-deps setting, and scripts-disabled install recipe on both sides:

```text
npm ci --ignore-scripts --audit=false --fund=false --update-notifier=false
  --install-strategy=hoisted --include=dev --include=optional --include=peer
  --bin-links=true --workspaces=false --legacy-peer-deps=false
```

The above is one command split for readability. Use `true` for legacy-peer-deps
when the package requires it. Give baseline npm explicit empty `--userconfig` and
`--globalconfig` files and a dedicated `--cache` directory. Warm that npm cache with
one untimed install. This avoids timing downloads against a prepared nmpool cache.

Alternate five warm npm installs and five restores, each into a fresh disposable
worktree at the recorded revision. Keep the npm download cache and nmpool cache
between runs. Time the entire command with a stopwatch, stopping on any nonzero
exit; exclude worktree creation and tests from install time. For example:

```powershell
$watch = [Diagnostics.Stopwatch]::StartNew()
# Run one npm ci or nmpool restore command here, checking $LASTEXITCODE.
$watch.Stop()
$watch.Elapsed.TotalSeconds
```

Report preparation cost separately. Compare medians, then consider how many
restores are needed to recover preparation cost. If restore is slower, the speed
hypothesis failed for this package. `apparent_bytes` is logical file size and
`physical_bytes_saved=null` is unknown; neither proves disk savings.

## Troubleshooting and cleanup

| Result | Meaning / next step |
|---|---|
| `destination_exists` | Use a fresh trial worktree; restore never replaces installs. |
| `cache_miss_or_incomplete` | Prepare with the same inputs and runtime; keep any incomplete entry for diagnosis. |
| `npm_cli_not_found` | Supply `--npm-cli 'C:\path\to\npm\bin\npm-cli.js'` on prepare and restore. |
| `npm_install_failed` | Read the reported staging `install.log`; both output streams are retained. |
| `cache_busy` / `destination_busy` | Wait for the other operation to finish; do not delete lock files to bypass it. |
| `artifact_mismatch...` | Stop using that entry; preserve it for diagnosis. There is no repair bypass. |
| `inputs_changed` / `toolchain_changed` | Stop concurrent edits/updates and retry with consistent inputs. |
| `unsupported...` / `...unsupported` | This workload is outside the initial profile; record the reason. |

Preparation retains staging, including npm download caches and logs, even after
success. Failed restores may leave `.nmpool-restore-*` directories in the trial
package. There is no GC. `.nmpool.lock` may remain after a successful restore.

After saving results, remove **only the worktrees created for this trial**, using
`git worktree remove` with their exact paths. Inspect a refusal before using
`--force`: installations/test outputs make disposable worktrees dirty. The cache
is separate and remains for inspection. You can manually delete the exact trial
cache/directory once no operation is using it and you no longer need its evidence.
Uninstall the executable with `cargo uninstall nmpool` if installed through Cargo.

## Bring back this result

```text
nmpool commit / CI artifact run:
Windows version, architecture, filesystem:
Node/npm versions:
Package revision and subdirectory:
Supported or exact refusal:
Baseline test command/result:
Restored test command/result:
Existing-destination refusal and independence check:
Preparation seconds:
Five warm npm seconds / median:
Five restore seconds / median:
Verdict: useful / slower / unsupported / inconclusive
```

Keep source, credentials and private package details on that machine. This
result needs only the necessary timings, versions and sanitized failure reason.
