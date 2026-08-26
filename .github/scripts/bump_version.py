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
import os
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


# `type(scope)!:` — the `!` is what makes it breaking, per Conventional
# Commits. Anchored to the start of the subject so a `!` anywhere else in
# the sentence is not a declaration.
BREAKING_SUBJECT = re.compile(r"^[a-z]+(\([^)]+\))?!:")
FEATURE_SUBJECT = re.compile(r"^feat(\([^)]+\))?:")
# 🔴 A FOOTER, which means the start of its own line — not the string
# appearing anywhere in the body. This workflow's own pull request
# described the rule in a bullet, the unanchored version matched that
# bullet, and the engine went from 0.2.44 to 1.0.0 on the first run.
BREAKING_FOOTER = re.compile(r"^BREAKING[ -]CHANGE:", re.MULTILINE)


def decide(title: str, body: str) -> str:
    """How far a PR with this title and body moves the version."""
    if BREAKING_SUBJECT.search(title) or BREAKING_FOOTER.search(body):
        return "major"
    if FEATURE_SUBJECT.search(title):
        return "minor"
    return "patch"


def self_test() -> int:
    cases = [
        ("feat!: rip out the old renderer", "", "major"),
        ("feat(render)!: rip out the old renderer", "", "major"),
        ("refactor!: rename every crate", "", "major"),
        ("fix: a footer declares it", "BREAKING CHANGE: the asset format moved", "major"),
        ("fix: a hyphenated footer", "BREAKING-CHANGE: same thing", "major"),
        # The regression this function exists for.
        ("ci: every pull request moves the engine version",
         "- `feat!:` / `BREAKING CHANGE:` -> major, `feat:` -> minor", "patch"),
        ("feat: contact shadows", "", "minor"),
        ("feat(lighting): the froxel grid", "", "minor"),
        ("fix: the cascade seam", "", "patch"),
        ("docs(book): the pipeline diagram", "", "patch"),
        ("chore: bump wgpu", "", "patch"),
        ("a title with no prefix at all", "", "patch"),
    ]
    failed = 0
    for title, body, want in cases:
        got = decide(title, body)
        if got != want:
            failed += 1
            print(f"FAIL {title!r} -> {got}, wanted {want}")
    print(f"{len(cases) - failed}/{len(cases)} decisions correct")
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--bump", choices=["major", "minor", "patch"])
    group.add_argument("--set", dest="exact", metavar="X.Y.Z")
    group.add_argument("--print", action="store_true", help="read the current version and stop")
    group.add_argument(
        "--decide",
        metavar="TITLE",
        help="print major/minor/patch for this PR title; the body comes from $PR_BODY",
    )
    group.add_argument("--self-test", action="store_true", help="check --decide against known cases")
    parser.add_argument(
        "--check",
        action="store_true",
        help="run `cargo metadata --no-deps` afterwards to prove the manifest still parses",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if args.decide:
        print(decide(args.decide, os.environ.get("PR_BODY", "")))
        return 0

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
