#!/usr/bin/env python3
"""Exercise an installed binary in a fresh temporary directory, without downloads."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="nmpool")
    args = parser.parse_args()
    binary = shutil.which(args.binary)
    if binary is None:
        parser.error("binary not found; pass --binary with its full path")
    binary = str(Path(binary).resolve())
    root = Path(tempfile.mkdtemp(prefix="nmpool-smoke-")).resolve()
    results = []

    def run(*command, expected=0):
        result = subprocess.run([binary, *map(str, command)], capture_output=True, text=True)
        results.append({"command": list(map(str, command)), "exit": result.returncode,
                        "stdout": result.stdout, "stderr": result.stderr})
        (root / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        if result.returncode != expected:
            raise RuntimeError(f"{command[0]} exited {result.returncode}, expected {expected}: {result.stderr}")
        return result

    def check(condition, message):
        if not condition:
            raise RuntimeError(message)

    print(f"Evidence and disposable fixture: {root}", flush=True)
    run("--version")
    package = root / "package"
    package.mkdir()
    manifest = {"name": "nmpool-smoke", "version": "1.0.0", "private": True}
    lock = {**manifest, "lockfileVersion": 3, "packages": {"": manifest}}
    (package / "package.json").write_text(json.dumps(manifest), encoding="utf-8")
    (package / "package-lock.json").write_text(json.dumps(lock), encoding="utf-8")
    cache = root / "cache"
    seed = json.loads(run("prepare", "--package", package, "--cache", cache).stdout)
    check(not (package / "node_modules").exists(), "prepare modified source install")
    run("restore", "--package", package, "--cache", cache)
    clean = json.loads(run("status", "--package", package).stdout)
    check(clean["state"] == "clean", "fresh restore was not clean")
    refused = run("restore", "--package", package, "--cache", cache, expected=1)
    check("destination_exists" in refused.stderr, "second restore failed for the wrong reason")
    (package / "node_modules" / "smoke-added.txt").write_text("private mutation\n", encoding="utf-8")
    drift = json.loads(run("status", "--package", package, expected=2).stdout)
    check(drift["state"] == "drifted", "consumer mutation was not detected")
    check(any(change["path"] == "smoke-added.txt" and change["change"] == "added"
              for change in drift["file_changes"]), "mutation missing from file changes")
    run("inspect", "--cache", cache, "--key", seed["key"])
    print("PASS: prepare, private restore, no-overwrite, drift detection, cache integrity.")
    print("This empty-package smoke test proves neither real-package compatibility nor speed.")
    print(f"Retained evidence: {root / 'results.json'}")


if __name__ == "__main__":
    main()
