#!/usr/bin/env python3
"""Move the workspace version, in `Cargo.toml` and in `Cargo.lock`.

Called by `.github/workflows/version.yml` once per pull request, and
usable by hand:

    python3 .github/scripts/bump_version.py --bump patch
    python3 .github/scripts/bump_version.py --set 0.3.0 --check

🔴 It edits `Cargo.lock` textually rather than running `cargo update`.
The runner has no warmed registry cache, so `cargo update --offline`
fails and the online one spends a minute resolving a graph in which
nothing but the workspace members moved. Every member takes its version
from `[workspace.package]`, so the whole change is one line per member —
and `cargo metadata --no-deps` is run afterwards to prove the result
still parses.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "Cargo.toml"
LOCKFILE = ROOT / "Cargo.lock"

# The version lives in the `[workspace.package]` table. Anchored to that
# header so a `version = ` inside `[workspace.dependencies]` is never the
# one that moves.
WORKSPACE_VERSION = re.compile(
    r"(?P<head>\[workspace\.package\][^\[]*?\bversion\s*=\s*\")(?P<version>[^\"]+)(?P<tail>\")",
    re.DOTALL,
)


def read_version() -> str:
    match = WORKSPACE_VERSION.search(MANIFEST.read_text())
    if not match:
        sys.exit("no version under [workspace.package] in Cargo.toml")
    return match.group("version")


def members() -> list[str]:
    """Workspace member crate names, read off the manifest's `members`."""
    text = MANIFEST.read_text()
    block = re.search(r"members\s*=\s*\[(.*?)\]", text, re.DOTALL)
    if not block:
        sys.exit("no `members` array in Cargo.toml")
    paths = re.findall(r"\"([^\"]+)\"", block.group(1))
    names = []
    for path in paths:
        manifest = ROOT / path / "Cargo.toml"
        name = re.search(r"^name\s*=\s*\"([^\"]+)\"", manifest.read_text(), re.M)
        if name:
            names.append(name.group(1))
    return names


def bumped(version: str, part: str) -> str:
    major, minor, patch = (int(n) for n in version.split("."))
    if part == "major":
        return f"{major + 1}.0.0"
    if part == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def write_manifest(new: str) -> None:
    text = MANIFEST.read_text()
    text = WORKSPACE_VERSION.sub(lambda m: m.group("head") + new + m.group("tail"), text, count=1)
    MANIFEST.write_text(text)


def write_lockfile(old: str, new: str, names: list[str]) -> int:
    """Retag every workspace member in `Cargo.lock`. Returns how many moved.

    Only a `[[package]]` block whose `name` is a member and whose version
    is the *old* one is touched, so a third-party crate that happens to
    share the version number is left alone.
    """
    if not LOCKFILE.exists():
        return 0
    text = LOCKFILE.read_text()
    moved = 0

    def retag(match: re.Match) -> str:
        nonlocal moved
        if match.group("name") in names and match.group("version") == old:
            moved += 1
            return f'{match.group("head")}{new}"'
        return match.group(0)

    pattern = re.compile(
        r'(?P<head>\[\[package\]\]\nname = "(?P<name>[^"]+)"\nversion = ")(?P<version>[^"]+)"'
    )
    LOCKFILE.write_text(pattern.sub(retag, text))
    return moved


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--bump", choices=["major", "minor", "patch"])
    group.add_argument("--set", dest="exact", metavar="X.Y.Z")
    group.add_argument("--print", action="store_true", help="read the current version and stop")
    parser.add_argument(
        "--check",
        action="store_true",
        help="run `cargo metadata --no-deps` afterwards to prove the manifest still parses",
    )
    args = parser.parse_args()

    current = read_version()
    if args.print:
        print(current)
        return 0

    new = args.exact if args.exact else bumped(current, args.bump)
    if new == current:
        print(f"already at {new}")
        return 0

    write_manifest(new)
    moved = write_lockfile(current, new, members())
    print(f"{current} -> {new} ({moved} lockfile entries)")

    if args.check:
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
            check=True,
            stdout=subprocess.DEVNULL,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
