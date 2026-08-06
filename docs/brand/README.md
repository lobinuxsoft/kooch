# Brand

The mark is a **teardrop tessellated into cluster-coloured meshlets**: the
myth is that Kóoch wept the sea into being, and the tessellation is the
engine's own visibility-buffer debug view. The logo is the renderer
looking at itself.

## Files

| File | What it is |
|---|---|
| `kooch_debug.svg` | **The formal logo.** Saturated, hue-separated clusters — the `MeshletIds` debug view. Editable source. |
| `kooch_myth.svg` | The same drop on one hue ramp, dark at the base to first light at the tip. |
| `gen_kooch_logo.py` | Generates both. **The mark is reproducible** — change the palette or the tessellation here, not in an editor. |
| `kooch_compare.png` | Both variants at three sizes. The reason there are two. |
| `kooch.png`, `logo_hi.png`, `bg.png` | Renders kept for reference. |
| `rendered/` | Sizes the repository actually consumes, produced from the SVGs. |

## Which variant, and why it is not a preference

**Debug where it is large. Myth where it is small.**

`gen_kooch_logo.py` says it about its own palette:

> *v1 — meshlet-debug palette: saturated, hue-separated. Reads as clusters
> up close, **flattens to rainbow mush below ~48px**.*
>
> *v2 — the myth: ... gives the mark a dominant value and **keeps it
> legible when small**.*

A title-bar icon is 16–32 px and a favicon is 16. At that size hue stops
discriminating and only value does, so the debug variant loses its
silhouette and stops reading as a drop at all. It is not that one variant
is better; it is that a mark needs a version per size, and both already
exist.

| Where | Variant | Size |
|---|---|---|
| README, wiki cover, social preview | debug | 256–1024 |
| Window icon (editor **and** any game) | myth | 64 |
| mdBook favicon | myth | 32 |

## Regenerating

```bash
python3 docs/brand/gen_kooch_logo.py          # rewrites the two SVGs
cd docs/brand
for s in 16 32 48 64 128 256; do resvg -w $s -h $s kooch_myth.svg  rendered/icon-$s.png; done
for s in 256 512 1024;        do resvg -w $s -h $s kooch_debug.svg rendered/logo-$s.png; done
cp rendered/icon-64.png ../../crates/kooch_window/icon/kooch-64.png
cp rendered/icon-32.png ../book/theme/favicon.png
```

⚠️ `crates/kooch_window/icon/kooch-64.png` is **embedded in the binary**
with `include_bytes!`, and a test asserts the corners are transparent and
the centre is not. Replace it with a square mark and that test fails,
which is the intent.

## Licence

Part of the project: **All Rights Reserved**, © 2025-2026 Matías Galarza
("Lobinux"). See [`LICENSE.md`](../../LICENSE.md).
