# UI

> In progress (#96). What exists today is the document and its tree; layout, text and drawing land
> phase by phase.

A UI is **one entity** with a `UiDocument`, the way Unity's UI Toolkit has a `UIDocument`. Its nodes
live inside the document, not in the hierarchy, so a menu of three hundred elements is still one
row in the scene.

| Field | What it is |
|---|---|
| `source` | The `.kooch_ui` file with the structure (#280) |
| `style` | The `.kooch_style` sheet with the look (#280) |
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

Ask for a node by name once and keep the handle: `tree.query("speed")`. Removing the `UiDocument`,
or despawning its entity, frees the tree.
