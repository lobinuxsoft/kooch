# Prototype textures

Kenney's *Prototype Textures* pack (CC0), 78 PNGs of 1024×1024 in six
colours, with a material beside each one under
`assets/materials/prototype/`.

They are here because a grey box tells you nothing about scale, and
because the renderer's own features cannot be judged without them: a
mip chain, a LOD bias and a sharpening pass all act on high-frequency
detail, and a scene of untextured surfaces has none to act on.

## ⚠️ The index is NOT the same pattern across colours

`dark/dark_texture_01` and `green/green_texture_01` are different patterns. The
pack numbers each colour's files independently, and the ones carrying
labelled reference geometry — STAIRS, DOOR, WINDOW, WALL — sit at
different indices in each folder: 11–13 in `green`, `orange`, `purple`
and `red`, 9–11 in `light`, 10–12 in `dark`.

The names are upstream's with the colour prefixed — `dark_texture_01`
rather than `texture_01` — because an asset picker shows the file name
and not the folder, and six files called `texture_01` are six identical
rows. The number is still upstream's. Renaming them to line up would mean
deciding by eye which of ten grid variants is "the same" as which, and a
mapping that is wrong in two places is worse than no mapping — pick from
the contact sheet in the book (`docs/book/src/images/prototype-textures.png`),
not from the number.

## Licence

CC0 1.0 Universal — public domain, no attribution required. Kenney asks
for credit anyway, and `NOTICE` at the repository root gives it.
Source: <https://kenney.nl/assets/prototype-textures>
