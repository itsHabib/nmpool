# Sharing qualification

Sharing is under construction. These commands gather evidence for the next stage;
`prepare` and `restore` still use private copies. No command here adopts, moves or
replaces an existing `node_modules`.

## Assess the package

```powershell
nmpool assess --package 'C:\src\your-repo\package' > assessment.json
# Exit 2 is expected: qualification is still required. Exit 1 means a failed read.
```

This reads package/lock metadata and ancestor configuration without running npm.
It reports the blockers together, including workspace context, install scripts,
registry/integrity gaps and runtime-write qualification. It does not approve
scripts, infer generator closure or establish the install's provenance. Do not
remove scripts or rewrite a lockfile to make an assessment pass.

The report omits dependency names, registry URLs, script bodies and configuration
values. Inspect it before posting publicly: local paths can identify your project.
Keep source files and credentials on the machine where the package lives.

## Test protection on the target volume

Choose an existing ordinary directory on the volume that would hold the pool.
The command creates its own disposable fixture there; it never protects that
parent or any existing installation.

```powershell
nmpool protection-probe --parent 'C:\nmpool-trials' > protection.json
# Exit 0: the fixture passed. Exit 2: a protection check failed. Exit 1: setup failed.
```

The probe attempts writes, creation, rename and deletion through a consumer link,
and checks whether the artifact root can be removed from its parent. Reading must
remain possible. Unix permission bits and Windows ACLs are tested by actual file
operations; Windows readonly attributes alone are insufficient.

A passing fixture establishes protection from ordinary accidental writes on that
volume. The owning user can deliberately change permissions. It does not qualify
an application, prove every production race safe or enable live sharing. Keep the
reported fixture and results until reviewed; cleanup instructions are included in
the report. Never recursively delete a live consumer link or shared target.

## Hand back evidence

Send the nmpool commit or CI artifact run, OS/filesystem, assessment and protection
reports, plus sanitized answers to these questions:

- What exact command installs this package, and does it depend on workspace siblings?
- Which generator inputs and commands produce required files such as Prisma clients?
- What formatting/build/test commands write under `node_modules`, and which support
  redirecting that state to a private directory outside it?

The next acceptance test needs two concurrent consumers, independent writable
state, unchanged dependency contents and the same application checks as the
baseline. A single shared `.cache` directory cannot satisfy that isolation test.
