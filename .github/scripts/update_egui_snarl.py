#!/usr/bin/env python3
"""Moves the in-tree egui-snarl fork (crates/egui_snarl) onto a newer upstream release.

Every file is merged three ways: upstream at the version the fork is based on, the fork as it is,
and upstream at the new version. Kóoch's changes carry over on their own; a conflict is only where
upstream changed the same lines, and it is left marked in the file.

    python3 .github/scripts/update_egui_snarl.py 0.12.0            # apply
    python3 .github/scripts/update_egui_snarl.py 0.12.0 --dry-run  # report only
"""

import argparse
import io
import pathlib
import re
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

FORK = pathlib.Path(__file__).resolve().parents[2] / "crates" / "egui_snarl"
CRATE = "https://static.crates.io/crates/egui-snarl/egui-snarl-{v}.crate"


def base_version() -> str:
    text = (FORK / "Cargo.toml").read_text()
    return re.search(r'^version\s*=\s*"([^"]+)"', text, re.M).group(1)


def fetch(version: str, into: pathlib.Path) -> pathlib.Path:
    request = urllib.request.Request(CRATE.format(v=version), headers={"User-Agent": "kooch"})
    with urllib.request.urlopen(request) as response:
        data = response.read()
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        archive.extractall(into, filter="data")
    return into / f"egui-snarl-{version}"


def sources(root: pathlib.Path) -> set[str]:
    return {str(p.relative_to(root)) for p in (root / "src").rglob("*.rs")}


def merge(ours: pathlib.Path, base: pathlib.Path, theirs: pathlib.Path) -> tuple[str, int]:
    """`git merge-file`: the merged text and how many conflicts it marked."""
    result = subprocess.run(
        ["git", "merge-file", "-p", "-L", "kooch", "-L", "upstream-old", "-L", "upstream-new",
         str(ours), str(base), str(theirs)],
        capture_output=True, text=True,
    )
    if result.returncode < 0:
        sys.exit(f"git merge-file failed on {ours}: {result.stderr}")
    return result.stdout, result.returncode


def dependencies(manifest: pathlib.Path) -> str:
    """Upstream's dependency tables — its workspace's too, which is where it pins egui."""
    text = manifest.read_text()
    blocks = re.findall(r"^\[(workspace\.dependencies|dependencies)\](.*?)(?=^\[|\Z)", text, re.S | re.M)
    return "\n".join(f"[{name}]\n{body.strip()}" for name, body in blocks)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    parser.add_argument("version", help="the upstream release to move onto, e.g. 0.12.0")
    parser.add_argument("--dry-run", action="store_true", help="report, write nothing")
    args = parser.parse_args()

    old = base_version()
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        base = fetch(old, tmp / "base")
        new = fetch(args.version, tmp / "new")
        conflicts = {}
        for name in sorted(sources(base) | sources(new) | sources(FORK)):
            ours, before, after = FORK / name, base / name, new / name
            if not after.exists():
                # Gone upstream. Ours stays only if it is ours — a test file moved out, say.
                print(f"  {name}: removed upstream{'' if before.exists() else ', kept (ours)'}")
                if before.exists() and not args.dry_run and ours.exists():
                    ours.unlink()
                continue
            empty = tmp / "empty"
            empty.write_text("")
            text, count = merge(ours if ours.exists() else empty, before if before.exists() else empty, after)
            if count:
                conflicts[name] = count
            if not args.dry_run:
                ours.parent.mkdir(parents=True, exist_ok=True)
                ours.write_text(text)

        manifest = (FORK / "Cargo.toml").read_text()
        if not args.dry_run:
            (FORK / "Cargo.toml").write_text(
                re.sub(r'^version\s*=\s*"[^"]+"', f'version = "{args.version}"', manifest, count=1, flags=re.M)
            )
        upstream_old = dependencies(base / "Cargo.toml.orig")
        upstream_new = dependencies(new / "Cargo.toml.orig")

    print(f"\negui-snarl {old} -> {args.version}{' (dry run)' if args.dry_run else ''}")
    if conflicts:
        print("Conflicts, marked in the files:")
        for name, count in conflicts.items():
            print(f"  {name}: {count}")
    else:
        print("No conflicts: every Kóoch change carried over.")
    if upstream_old != upstream_new:
        print("\nUpstream's [dependencies] changed; carry it into crates/egui_snarl/Cargo.toml by hand:")
        print(upstream_new)
    print("\nThen: bump egui in the workspace if upstream moved it, `cargo check -p egui-snarl`,")
    print("run the Shader Graph tests, and update crates/egui_snarl/KOOCH.md.")
    sys.exit(1 if conflicts else 0)


if __name__ == "__main__":
    main()
