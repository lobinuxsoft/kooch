#!/usr/bin/env python3
"""Generate the Kooch logo: a teardrop tessellated into meshlet-style clusters.

The silhouette is the myth (Kooch wept the sea into being); the tessellation is
the engine (cluster-coloured meshlets, as seen in a visibility-buffer debug view).
Triangles are grouped into clusters that share a colour, which is what actually
makes it read as "meshlets" rather than as random confetti.
"""
import math

SIZE = 512
# Teardrop: point at the top, circular lobe at the bottom.
TIP = (256.0, 34.0)
LOBE_C = (256.0, 332.0)
LOBE_R = 154.0

DROP_PATH = (
    f"M{TIP[0]},{TIP[1]} "
    f"C{TIP[0]},{TIP[1]} {LOBE_C[0]+LOBE_R},{LOBE_C[1]-118} "
    f"{LOBE_C[0]+LOBE_R},{LOBE_C[1]} "
    f"A{LOBE_R},{LOBE_R} 0 1,1 {LOBE_C[0]-LOBE_R},{LOBE_C[1]} "
    f"C{LOBE_C[0]-LOBE_R},{LOBE_C[1]-118} {TIP[0]},{TIP[1]} {TIP[0]},{TIP[1]} Z"
)

# v1 — meshlet-debug palette: saturated, hue-separated. Reads as clusters up
# close, flattens to rainbow mush below ~48px.
PALETTE_DEBUG = [
    "#F65C8A", "#FF8A3D", "#FFC94A", "#8CE06B",
    "#2FD6B0", "#35B8F5", "#6C7BF7", "#B963F0",
    "#FF6B6B", "#4ADE80", "#22D3EE", "#A78BFA",
]

# v2 — the myth: darkness at the base, sea through the middle, first light at
# the tip. Ordered dark -> light so vertical position picks the ramp index,
# which gives the mark a dominant value and keeps it legible when small.
PALETTE_MYTH = [
    "#0A1F38", "#0E2C4E", "#123A63", "#164A78",
    "#1A5C88", "#1E7194", "#22899C", "#2AA3A2",
    "#3CBBA6", "#5FD3B0", "#8FE6C6", "#C6F3DE",
]
# Sparse warm accents: the light the wind let through. Used on a few tip-side
# triangles only — more than a handful and it stops reading as "first light".
ACCENTS = ["#FFC46B", "#FFA94D", "#FFE0A3"]

COLS, ROWS = 9, 11
PAD = 12.0


def rnd(i: int, j: int, salt: int) -> float:
    """Deterministic pseudo-random in [-1, 1]. No RNG seeds, no surprises."""
    h = math.sin(i * 127.1 + j * 311.7 + salt * 74.7) * 43758.5453
    return (h - math.floor(h)) * 2.0 - 1.0


def grid_point(i: int, j: int) -> tuple[float, float]:
    x0, x1 = PAD, SIZE - PAD
    y0, y1 = PAD, SIZE - PAD
    x = x0 + (x1 - x0) * i / COLS
    y = y0 + (y1 - y0) * j / ROWS
    # Interior points get jittered; the clip handles the outside anyway.
    jx = (x1 - x0) / COLS * 0.30
    jy = (y1 - y0) / ROWS * 0.30
    return (x + rnd(i, j, 1) * jx, y + rnd(i, j, 2) * jy)


def cluster_id(i: int, j: int) -> int:
    """Group neighbouring cells so several triangles share one colour."""
    base = (i // 2) * 7 + (j // 2) * 13
    wobble = 1 if rnd(i // 2, j // 2, 9) > 0.35 else 0
    return base + wobble


pts = {(i, j): grid_point(i, j) for i in range(COLS + 1) for j in range(ROWS + 1)}


def build(mode: str) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for j in range(ROWS):
        for i in range(COLS):
            a, b, c, d = pts[(i, j)], pts[(i + 1, j)], pts[(i + 1, j + 1)], pts[(i, j + 1)]
            # Flip the diagonal on alternating cells so the mesh is not woven.
            if (i + j) % 2 == 0:
                quads = [(a, b, c), (a, c, d)]
            else:
                quads = [(a, b, d), (b, c, d)]
            cid = cluster_id(i, j)
            for k, tri in enumerate(quads):
                if mode == "debug":
                    colour = PALETTE[(cid + k * (1 if rnd(i, j, 3) > 0 else 0)) % len(PALETTE)]
                else:
                    # Vertical position drives the ramp: dark at the base,
                    # light at the tip. Cluster jitter keeps it from banding.
                    cy = sum(p[1] for p in tri) / 3.0
                    t = 1.0 - min(max((cy - 30.0) / (SIZE - 60.0), 0.0), 1.0)
                    idx = int(t * (len(PALETTE) - 1) + rnd(i, j, 5 + k) * 1.4)
                    idx = min(max(idx, 0), len(PALETTE) - 1)
                    colour = PALETTE[idx]
                    # A few warm triangles near the tip: the first light.
                    if t > 0.78 and rnd(i, j, 11 + k) > 0.55:
                        colour = ACCENTS[(cid + k) % len(ACCENTS)]
                pt = " ".join(f"{x:.1f},{y:.1f}" for x, y in tri)
                out.append((pt, colour))
    return out


MODE = __import__("sys").argv[1] if len(__import__("sys").argv) > 1 else "myth"
PALETTE = PALETTE_DEBUG if MODE == "debug" else PALETTE_MYTH
tris = build(MODE)

triangles = "\n".join(
    f'      <polygon points="{p}" fill="{c}"/>' for p, c in tris
)

svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" role="img" aria-label="Kooch">
  <title>Kooch</title>
  <defs>
    <clipPath id="drop">
      <path d="{DROP_PATH}"/>
    </clipPath>
    <linearGradient id="depth" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#000000" stop-opacity="0.34"/>
      <stop offset="0.45" stop-color="#000000" stop-opacity="0"/>
      <stop offset="1" stop-color="#000000" stop-opacity="0.30"/>
    </linearGradient>
  </defs>

  <g clip-path="url(#drop)">
{triangles}
    <!-- Cluster seams: thin dark edges, the way a meshlet debug view reads. -->
    <g fill="none" stroke="#070B14" stroke-opacity="0.30" stroke-width="1.6">
{chr(10).join(f'      <polygon points="{p}"/>' for p, _ in tris)}
    </g>
    <rect width="{SIZE}" height="{SIZE}" fill="url(#depth)"/>
  </g>

  <!-- Silhouette outline: this is what survives at 16px. -->
  <path d="{DROP_PATH}" fill="none" stroke="#070B14" stroke-width="10" stroke-linejoin="round"/>
</svg>
"""

out = f"/tmp/claude-1000/-var-home-lobinux/76474583-5060-4ce7-8119-e4f49b8ad19c/scratchpad/kooch_{MODE}.svg"
with open(out, "w") as f:
    f.write(svg)
print(f"wrote {out} ({len(tris)} triangles, mode={MODE})")
