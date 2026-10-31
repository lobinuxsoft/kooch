# UI

> In progress (#96). What exists today is the document and its tree; layout, text and drawing land
> phase by phase.

A UI is **one entity** with a `UiDocument`, the way Unity's UI Toolkit has a `UIDocument`. Its nodes
live inside the document, not in the hierarchy, so a menu of three hundred elements is still one
row in the scene.

| Field | What it is |
|---|---|
| `source` | The `.kxml` file with the structure (#280) |
| `style` | The `.kcss` sheet with the look (#280) |
| `mode` | **Overlay** draws over the screen; **World** places it in the scene at the entity |

Enable it with the engine's `ui` feature.

## Units

An Overlay is laid out in a **logical canvas 1080 units tall**, as wide as the screen's aspect
makes it: 1920×1080 on a 16:9 screen, 1728×1080 on a 16:10 handheld. The same UI takes the same
share of any screen, whatever its resolution or the OS scale factor — Unity's *Scale With Screen
Size*, matching height.

A World document is sized by its content for now.

Layout runs in `PostUpdate`, after gameplay, and only when the tree or the screen changed. Read a
node's place with `tree.rect(node)`: top-left origin, y down, in canvas units.

## Building a tree from code

Until the file formats arrive, a game builds the tree itself and hands it to `UiTrees`:

```rust
use kooch::prelude::*;
use kooch::kooch_ui::taffy::Style;

let mut tree = UiTree::new(Style::default());
let speed = tree.push(UiTree::ROOT, Style::default()).unwrap();
tree.set_name(speed, "speed");
resources.get_mut::<UiTrees>().unwrap().insert(document_entity, tree);
```

## Text

```rust
tree.set_text(speed, "Velocidad: 12,5 m/s", 32.0);
```

Text is shaped with **Inter**, which the engine ships (SIL Open Font License, in
`crates/kooch_ui/assets/fonts/`); a glyph Inter lacks falls back to the system's sans-serif. The
node is measured from its text, so a label pushes its siblings in a row. Ligatures and accents come
from the shaper on their own.

A game writing into a document built from files names the node and keeps the size the `.kcss`
gave it:

```rust
resources.get_mut::<UiTrees>().unwrap().write_text(document, "clicks", "Clicks: 3");
```

**Fira Code** ships too, with its programming ligatures (`=>`, `!=`): `tree.set_font(node, "Fira
Code")` after `set_text`.

Sizes are in canvas units, and glyphs are rasterized at the screen's **physical** size: text stays
sharp at 720p and at 1440p.

## Painting

```rust
tree.set_background(panel, [0.08, 0.09, 0.12, 0.85]);
tree.set_border_color(panel, [1.0, 0.55, 0.1, 1.0]); // width is the layout's `Style::border`
tree.set_radius(panel, 18.0);
tree.set_text_color(label, [0.55, 0.85, 1.0, 1.0]);
```

Colours are sRGB, as a colour picker gives them. An Overlay draws over the finished frame, after
the post-process.

In the editor's Play the game runs in another process with no window: it lays the UI out at the
Game View's size, which the editor sends it, and the Game View draws what it built. Resize the panel
and the layout follows. Only Overlay documents draw for now, here and in the game.

Ask for a node by name once and keep the handle: `tree.query("speed")`. Removing the `UiDocument`,
or despawning its entity, frees the tree.
