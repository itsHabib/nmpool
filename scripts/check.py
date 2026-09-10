"""Run the same locked checks on macOS, Windows, and CI (Python 3 required)."""

import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
COMMANDS = [
    ["cargo", "fmt", "--all", "--", "--check"],
    ["cargo", "clippy", "--locked", "--all-targets", "--all-features", "--", "-D", "warnings"],
    ["cargo", "test", "--locked", "--all-features"],
    ["cargo", "doc", "--locked", "--all-features", "--no-deps"],
]


def main():
    env = os.environ.copy()
    env["RUSTDOCFLAGS"] = (env.get("RUSTDOCFLAGS", "") + " -D warnings").strip()
    for command in COMMANDS:
        print("+ " + " ".join(command), flush=True)
        subprocess.run(command, cwd=ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
