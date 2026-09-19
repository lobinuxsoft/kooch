#!/usr/bin/env python3
"""Checks a split moved code without changing it: the multiset of code TOKENS across the given
paths at BASE equals the working tree's, so rustfmt reflowing a signature is not a change.
Ignored: comments, `use`/`mod` items, visibility, `#[cfg(test)]`, and trailing commas."""
import collections, re, subprocess, sys, pathlib

base, roots = sys.argv[1], sys.argv[2:]
TOKEN = re.compile(r'"(?:\\.|[^"\\])*"|[A-Za-z_][A-Za-z0-9_]*|\d[\w.]*|::|->|=>|[^\s\w]')

def tokens(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    text = re.sub(r"//[^\n]*", " ", text)
    text = re.sub(r"\b(pub(\s*\([^)]*\))?\s+)?(use|mod)\b[^;{]*(\{[^;]*\})?[^;]*;", " ", text)
    text = re.sub(r"#\[cfg\(test\)\]", " ", text)
    text = re.sub(r"\bpub(\s*\([^)]*\))?\s+", " ", text)
    out = TOKEN.findall(text)
    return [t for i, t in enumerate(out) if not (t == "," and i + 1 < len(out) and out[i + 1] in ")]}")]

def at(ref):
    c = collections.Counter()
    files = subprocess.run(["git", "ls-tree", "-r", "--name-only", ref, "--", *roots],
                           capture_output=True, text=True, check=True).stdout.split()
    for f in files:
        if f.endswith(".rs"):
            c.update(tokens(subprocess.run(["git", "show", f"{ref}:{f}"], capture_output=True, text=True).stdout))
    return c

def now():
    c = collections.Counter()
    for root in map(pathlib.Path, roots):
        for f in ([root] if root.is_file() else root.rglob("*.rs")):
            c.update(tokens(f.read_text()))
    return c

before, after = at(base), now()
gone, new = before - after, after - before
print("removed:", dict(gone.most_common(30)))
print("added:  ", dict(new.most_common(30)))
