#!/usr/bin/env python3
"""Moves line ranges of a Rust file into child modules, unchanged.

split.py FILE PLAN.json — PLAN: [{"name": "record", "doc": "...", "ranges": [[a, b], ...],
"impl": "PageRasterizer" | null}]. Ranges are 1-based, inclusive, and must include each item's doc
comments. The child gets `use super::*;`; an `impl` wraps its ranges in `impl T { … }` (ranges then
hold methods only). The parent gets `mod name;` before its first `#[cfg(test)]`, or at the end."""
import json, pathlib, sys

path = pathlib.Path(sys.argv[1]); plan = json.load(open(sys.argv[2]))
lines = path.read_text().splitlines(keepends=True)
taken = set()
child_dir = path.parent if path.name in ("mod.rs", "lib.rs") else path.with_suffix("")
child_dir.mkdir(exist_ok=True)
for part in plan:
    body = []
    for a, b in part["ranges"]:
        for i in range(a - 1, b):
            assert i not in taken, f"line {i+1} taken twice"
            taken.add(i)
        chunk = lines[a - 1:b]
        body.append("".join(chunk).rstrip("\n") + "\n")
    text = f"//! {part['doc']}\n\nuse super::*;\n\n"
    joined = "\n".join(body)
    if part.get("impl"):
        text += f"impl {part['impl']} {{\n{joined}}}\n"
    else:
        text += joined
    out = child_dir / f"{part['name']}.rs"
    assert not out.exists(), f"{out} exists"
    out.write_text(text)
kept = [l for i, l in enumerate(lines) if i not in taken]
src = "".join(kept)
# Collapse runs of blank lines left by the moves.
while "\n\n\n" in src:
    src = src.replace("\n\n\n", "\n\n")
mods = "".join(f"mod {p['name']};\n" for p in plan)
marker = "#[cfg(test)]\nmod tests;"
if marker in src:
    src = src.replace(marker, mods + "\n" + marker, 1)
else:
    src = src.rstrip("\n") + "\n\n" + mods
path.write_text(src)
print(f"{path}: {len(kept)} lines left")
