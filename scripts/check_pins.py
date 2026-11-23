#!/usr/bin/env python3
"""Every pin this project's supply chain rests on, checked before a build.

    python3 -I scripts/check_pins.py

- every action in .github/workflows/ and .github/actions/ is named by a full commit SHA with its
  version in a comment (`uses: owner/repo@<40 hex> # vX.Y.Z`); local
  workflows (`./...`) are this repository's own;
- the axross-amp submodule is a commit (a gitlink), its URL the desktop
  program's repository;
- Cargo.toml takes axross-amp from that submodule (a path, no git or
  registry source that could move);
- this project's Cargo.lock holds no crate or version the desktop's lock
  does not (the reviewed baseline is the desktop's).

Exits 1 with one line per finding.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
USES = re.compile(r"^\s*(?:-\s*)?uses:\s*(\S+)(.*)$")
PINNED = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}$")
VERSION = re.compile(r"#\s*v\d+(\.\d+)*\b")
SUBMODULE_URL = "https://github.com/c0decave/axross-amp"
PACKAGE = re.compile(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"')


def workflows(problems):
    github = ROOT / ".github"
    files = sorted(github.glob("workflows/*.y*ml")) + sorted(github.glob("actions/*/action.y*ml"))
    for path in files:
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            found = USES.match(line)
            if not found:
                continue
            target, rest = found.group(1), found.group(2)
            where = f"{path.relative_to(ROOT)}:{number}"
            if target.startswith("./"):
                continue
            if not PINNED.match(target):
                problems.append(f"{where}: {target} is not pinned by a commit SHA")
            elif not VERSION.search(rest):
                problems.append(f"{where}: {target} has no version comment (# vX.Y.Z)")


def submodule(problems):
    modules = ROOT / ".gitmodules"
    text = modules.read_text(encoding="utf-8") if modules.exists() else ""
    if f"url = {SUBMODULE_URL}" not in text or "path = axross-amp" not in text:
        problems.append(f".gitmodules: axross-amp is not the submodule of {SUBMODULE_URL}")
    listed = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "--stage", "--", "axross-amp"],
        capture_output=True, text=True, check=False,
    ).stdout.split()
    # (mode 160000: a commit of another repository, not files of this one)
    if not listed or listed[0] != "160000":
        problems.append("axross-amp is not a submodule (no gitlink in the index)")
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    if not re.search(r'^axross-amp\s*=\s*\{\s*path\s*=\s*"axross-amp"\s*\}\s*$', manifest, re.M):
        problems.append('Cargo.toml: axross-amp is not `{ path = "axross-amp" }`')


def locks(problems):
    desktop = ROOT / "axross-amp" / "Cargo.lock"
    if not desktop.exists():
        problems.append("axross-amp/Cargo.lock is missing (git submodule update --init)")
        return
    ours = set(PACKAGE.findall((ROOT / "Cargo.lock").read_text(encoding="utf-8")))
    theirs = set(PACKAGE.findall(desktop.read_text(encoding="utf-8")))
    for name, version in sorted(ours - theirs):
        if name != "axross-amp-android":
            problems.append(
                f"Cargo.lock: {name} {version} is not in the desktop's lock "
                "(no reviewed baseline entry covers it)"
            )


def main():
    problems = []
    workflows(problems)
    submodule(problems)
    locks(problems)
    for problem in problems:
        print(problem)
    if problems:
        return 1
    print("pins: the actions, the submodule and the lock are as they should be")
    return 0


if __name__ == "__main__":
    sys.exit(main())
