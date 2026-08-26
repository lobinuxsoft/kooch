#!/usr/bin/env python3
"""Derive the crate layering from the workspace, for `docs/book`'s crate graph.

    python3 .github/scripts/crate_layers.py           # print the two tables
    python3 .github/scripts/crate_layers.py --check    # exit 1 if the page drifted

A crate's layer is its **longest path to a crate with no internal
dependencies**. That is a property of the `Cargo.toml` files, not an
opinion, and the page that documented it by hand had drifted by three
layers and a whole crate — which is what this exists to stop.

`--check` is deliberately shallow: it compares the crate *names* per layer
against the table in the page, and says nothing about the prose. A layer
that moves is a fact; whether the paragraph explaining it still reads true
is a judgement, and a script that claimed to check that would be lying.
"""

import argparse
import functools
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PAGE = ROOT / "docs/book/src/architecture/crate-graph.md"
# An example, not a crate of the engine. It is a workspace member so that
# `cargo test` builds it — a plugin that stops compiling is a broken ABI.
NOT_ENGINE = {"example_plugin"}


def graph() -> dict[str, list[str]]:
    """Internal dependencies per workspace crate, normal deps only."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    packages = {p["name"]: p for p in json.loads(out.stdout)["packages"]}
    names = set(packages) - NOT_ENGINE
    return {
        name: sorted(
            {d["name"] for d in packages[name]["dependencies"] if d["name"] in names and d["kind"] is None}
        )
        for name in names
    }


def layers(deps: dict[str, list[str]]) -> dict[int, list[str]]:
    @functools.lru_cache(None)
    def depth(name: str) -> int:
        return 0 if not deps[name] else 1 + max(depth(d) for d in deps[name])

    found: dict[int, list[str]] = {}
    for name in deps:
        found.setdefault(depth(name), []).append(name)
    return {layer: sorted(names) for layer, names in sorted(found.items())}


def documented() -> dict[int, list[str]]:
    """The crates each `**L<n> · …**` row of the page's table lists."""
    found: dict[int, list[str]] = {}
    for line in PAGE.read_text().splitlines():
        row = re.match(r"\|\s*\*\*L(\d+)[^|]*\|([^|]*)\|", line)
        if row:
            found[int(row.group(1))] = sorted(re.findall(r"`(kooch[a-z_]*)`", row.group(2)))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="compare against the page and exit 1 on drift")
    args = parser.parse_args()

    deps = graph()
    found = layers(deps)

    if args.check:
        page = documented()
        drift = [
            f"L{layer}: workspace {found.get(layer, [])} vs page {page.get(layer, [])}"
            for layer in sorted(set(found) | set(page))
            if found.get(layer, []) != page.get(layer, [])
        ]
        if drift:
            print("crate-graph.md has drifted:")
            print("\n".join("  " + line for line in drift))
            return 1
        print(f"crate-graph.md matches the workspace ({len(deps)} crates, {len(found)} layers)")
        return 0

    print(f"{len(deps)} crates in {len(found)} layers\n")
    for layer, names in found.items():
        print(f"| **L{layer}** | " + ", ".join(f"`{n}`" for n in names) + " |")
    print()
    for name in sorted(deps):
        print(f"| `{name}` | " + (", ".join(f"`{d}`" for d in deps[name]) or "—") + " |")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
