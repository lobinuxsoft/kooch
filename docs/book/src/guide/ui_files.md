# UI files

> 🚧 `bind:value` reaches components on the document's entity; a resource as its target is not
> live yet.

A `UiDocument` points at two files, split like Unity's UXML and USS:

| File | Says | Format |
|---|---|---|
| `.kxml` | which nodes, nested how, what they do | XML |
| `.kcss` | how they look | a subset of CSS |

The document is built from them in whichever process shows it: the editor's own with Play stopped,
so you design without running the game, and the game's in Play.

Create the two files under `assets/`, pick the `.kxml` as the document's **Source** and the `.kcss`
as its **Style**. The markup never names a sheet: the same `.kxml` can wear another `.kcss` on
another document. A document with no Style uses the engine's default sheet.
Save either file and the Game View picks it up — no Play needed, and in Play too. Saving the
`.kcss` restyles the live document: text the game wrote, values, hidden nodes, classes and focus all
stay. Saving the `.kxml` (or a template it uses) rebuilds it; focus follows a node that has a
`name`. A document with no Source is left to code, as in [UI](./ui.md).

Both are plain XML and CSS, so any editor reads them as such once it knows the extension. Opening
the project in the Kóoch editor adds `"*.kxml": "xml"` and `"*.kcss": "css"` to
`.vscode/settings.json`. The `xmlns` line on `<ui>` is what keeps an XML linter quiet about `on:`
and `bind:`; the engine declares them itself when it is missing. A property that is Kóoch's own
carries the `-kooch-` prefix, which CSS linters skip as vendor-specific.

### Layers

One `UiDocument` is one layer: the HUD, the pause menu and a dialog are three entities, each with
its own focus and its own input actions. Screens that replace each other — main menu, options,
credits — are one layer that swaps its **Source**. Find a layer by the data component on its entity
(`Query<(&UiDocument, &PauseMenu)>`), never by a name in the file.

**Hidden** keeps a layer's tree but stops drawing it, picking it and reading its actions; the focus
it held is dropped, with `on:blur`. Showing it again hands focus to its `autofocus`, so opening a
menu lands on its first button. Turning a layer off by clearing its Source instead rebuilds it from
the files and loses whatever the game wrote into it.

Many panels in one document work too, and are the cheaper choice for parts that always show
together. Layers cost little: all of them draw in one call, and a layer that did not change does
not lay out or shape text again; each still reads its two files' dates once a frame.

## Markup

```xml
<?xml version="1.0" encoding="UTF-8"?>
<ui xmlns:on="urn:kooch:on" xmlns:bind="urn:kooch:bind">
  <panel name="menu" class="card">
    <label class="title">Pausa</label>
    <button name="resume" on:click="game.resume"><label>Continuar</label></button>
    <slider name="volume" min="0" max="1" bind:value="AudioSettings.volume"/>
  </panel>
</ui>
```

| Element | Is |
|---|---|
| `ui` | The root. `style` names the style sheet, relative to this file. |
| `panel` | A box that lays out its children. |
| `label` | Text: the element's content. |
| `button` | A panel that can be clicked and focused. |
| `toggle` | A checkbox; its value is `0` or `1`. `checked="true"` starts it on. |
| `slider` | A track with a thumb; its value is an `f32` between `min` and `max` (default `0`–`1`), starting at `value`. `step` snaps it; without one the pad moves it a tenth of the range. |
| `use` | Another `.kxml`, instanced here. |

Every element takes `name` (what `tree.query` finds, unique in the document) and `class` (any
number, space separated).

Code reads and changes a document through its tree, without touching entities:

```rust
let tree = resources.get_mut::<UiTrees>().unwrap().get_mut(document).unwrap();
let coins: Vec<UiNode> = tree.with_class("coin").collect();
tree.set_class(coins[0], "taken", true); // the sheet's `.taken` rules apply this frame
let hint = tree.query("hint").unwrap();
tree.set_visible(hint, false); // `display: none`: no space, no draw, no pointer, no focus
```

Showing a node again gives it back the display the sheet sets. Hiding a whole layer is
`UiDocument.hidden`.

### Templates

`use` instances a file. `<set node="…" attribute="…"/>` inside it overrides one attribute of a named
node of the instance — the same button file used twice with two texts. Attributes on `use` itself
(`name`, `class`, `on:*`) apply to the instance's root. A `style` on either goes on top of the
template's: only what it writes changes, so moving a `<use>` in the UI Builder keeps the rest, and
unsetting a property gives the template's value back.

```xml
<use src="menu_button.kxml" name="quit" class="danger" on:click="game.quit">
  <set node="label" text="Salir"/>
</use>
```

A template is a `.kxml` like any other, whose `<ui>` holds exactly one element — the instance:

```xml
<ui>
  <button class="menu-button"><label name="label">Botón</label></button>
</ui>
```

`src` is relative to the file that uses it, and a template may use another. `<set>` takes any
attribute: `text`, `class` (replaces the node's classes), `on:click`, `disabled`… The instance is
styled by the document's sheet, which lets one button file follow each menu's look, and saving the
template rebuilds every document that uses it. `<ui sheet="bar.kcss">` makes a template look the same
everywhere instead: it wears that sheet, read beside it, or the engine's with `sheet="default"`, and
the sheet's `ui` rule reaches it by inheritance. A template inside it keeps its own if it names one.
The UI Builder picks it on `<ui>`; opened on its own, a file wears what the canvas picks. Names inside an instance stay as written, so two instances both have a
`label`; reach one through the instance's own name.

### From code

A menu whose items are not known until the game runs — an inventory, a level list — is filled from
code. `ui_instance` instances markup under a node of a built document, with the same `use` and
`set` a file writes, `src` relative to the document's file:

```rust
let Some(list) = resources.get::<UiTrees>()
    .and_then(|trees| trees.get(document)?.query("items")) else { return };
for (index, item) in items.iter().enumerate() {
    let markup = format!(
        r#"<use src="menu_button.kxml" data-index="{index}" on:click="items.pick">
             <set node="label" text="{}"/>
           </use>"#,
        item.name,
    );
    ui_instance(resources, document, list, &markup)?;
}
```

Each instance clicks with its own action, or all share one and the handler reads which:
`tree.attribute(event.node, "data-index")`. `UiTree::remove(node)` takes an item out with
everything under it. It builds the tree again, so a `UiNode` kept from before is stale; what the
game wrote (texts, values, scroll) stays.

Instances are the game's, not the file's: saving the `.kxml` builds the document again without
them, so a system that fills a menu fills it again when its items are missing.

### Events and bindings

`on:<event>="action"` names an action, it does not point at a function: the file stays data, the
editor can list the actions on offer, and it works the same when the game runs in another process.

| Event | On | Carries |
|---|---|---|
| `on:click` | `button` | — |
| `on:change` | `toggle`, `slider` | the new value |
| `on:focus` / `on:blur` | anything focusable | — |
| `on:cancel` | the focused node or any ancestor; `<ui on:cancel>` answers for the document | — |

`autofocus="true"` gives a node focus when the document is built — a menu's first button.
`tabindex="-1"` keeps a node with an action out of focus — a HUD button the pad should not reach.
`disabled="true"` keeps a node from clicking and from taking focus, and selects `:disabled`; it still
stops the pointer, so the click does not reach the game. Code turns it on and off with
`UiTree::set_disabled(node, on)`.

`bind:value="Type.field"` keeps a `toggle` or `slider` in step with a field of a component on the
document's entity, both ways: moving the slider writes the field, and changing the field moves the
slider. `Type` is the reflected component's name; the field may be a float, an integer or a `bool`.
The UI holds no value of its own that matters: the field does.

A label's text can name fields too. Each `{Type.field}` hole shows the field's value, and the text is
laid out again only when one of them changes:

```xml
<label name="hp_label">[HP {Hud.hp:>3%}]</label>
<label>Coins: {Inventory.coins}</label>
```

After the `:` goes a format spec, as in Python: fill and alignment (`<`, `^`, `>`), width (`>3`,
`*^8`) and precision (`.1`). `%` shows a 0–1 value as a whole percent (`.1%` keeps a decimal).
Numbers align right and text aligns left. `{{` and `}}` write a brace. A hole that names no field
stays as written, and the log says which name it could not find. A label with no `{Type.field}` is
plain text.

A click or `Submit` flips a toggle; a slider follows a held press, and a focused one moves with
left/right on the pad or keys (up/down still move focus). Each sends `on:change`, its event's
`value` the new one. Code reads and sets a control with `UiTree::value` / `set_value`; setting from
code sends no `on:change`.

A system hears the UI the way it hears input, by reading events. They are read the frame after the
click:

```rust
fn pause_menu(resources: &mut Resources) {
    let Some(events) = resources.get::<Events<UiEvent>>() else { return };
    for event in events.read() {
        if event.action == "game.quit" {
            // ...
        }
    }
}
```

Or the project registers the action once, from a `Startup` system, and the UI runs it the frame the
click lands:

```rust
fn quit(resources: &mut Resources, _event: &UiEvent) { /* ... */ }

fn register_actions(resources: &mut Resources) {
    if let Some(actions) = resources.get_mut::<UiActions>() {
        actions.add("game.quit", quit);
    }
}
```

A click over the UI does not reach the game: while the pointer is over a node with an action or a
visible background, input actions read mouse buttons and scroll as idle. A transparent layout panel
lets the click through. Raw reads (`is_mouse_pressed`) still see everything, so a system that reads
them asks `UiPointer::over` first. A press that started over the game stays the game's until it is
released, so dragging the camera across a panel does not drop it.

With the cursor captured for mouse look there is no pointer, and nothing is picked.

### Input

The UI reads **Input Actions**, the same `.inputaction` assets the game reads — there is no second
input system. A `UiDocument` has six action fields; an empty one reads the engine's default:

| Field | Type | Default |
|---|---|---|
| **Point** | Vector2 | the mouse (`Pointer X`/`Pointer Y`) |
| **Click** | Button | left mouse button |
| **Scroll** | Vector2 | the mouse wheel |
| **Navigate** | Vector2 | arrows, d-pad, left stick |
| **Submit** | Button | Enter, Space, South |
| **Cancel** | Button | Escape, East |

A press focuses the node with an action it lands on, and a press on the game drops focus. With
focus, **Navigate** moves it to the nearest focusable node that way — once when pushed, then
repeating after half a second — **Submit** clicks it, and **Cancel** sends the nearest `on:cancel`.
Only the document holding focus reads Navigate, Submit and Cancel, so a HUD nobody focused never
takes the stick; the game still reads them too, so a button that keeps focus answers the jump key.
`tabindex="-1"` is for that. **Scroll** moves the innermost `overflow: scroll` panel under the
pointer, 40 canvas units a wheel line.

A system answers an action by name: `ui_actions(resources, "hud.count")` is every document that sent
it this frame, once per send.

> Live today: `on:click`, `on:focus`/`on:blur`, `on:cancel`, `on:change`, `bind:value`,
> `bind:param`, `{Type.field}` text and pad navigation on Overlay documents.

## Style

An element's own `style="…"` holds declarations like a rule's block, without a selector, and beats
every rule of the sheet, as in CSS. `var()` and `url()` images belong in the sheet, not inline.

```css
:root { --accent: #ff8a1f; }

.card {
  flex-direction: column;
  gap: 12px;
  padding: 24px;
  background-color: rgba(16, 16, 20, 0.85);
  border: 3px var(--accent);
  border-radius: 18px;
}

.menu-button:hover { background-color: #3a3a46; }
.menu-button.danger #label { color: #ff5a5a; }
```

**Units.** `px` is a canvas unit — the screen is 1080 of them tall, as in [UI](./ui.md) — and `%`
is of the parent. Nothing else. So `px` follows the screen's height, and a width in `%` its width: a
node sized `%` on both axes changes shape with the aspect ratio. `height: 30%; aspect-ratio: 16 / 9`
holds it, with the other side left `auto`.

`font-size: 50%` is half the parent's height, so text follows its box when that resizes. The parent
needs a height of its own: one sized by the text it holds has nothing to follow.

**Selectors.** Element (`slider`), class (`.card`), name (`#label`), the descendant combinator (a
space), and the states `:hover`, `:active`, `:focus`, `:checked`, `:disabled`. Specificity and
precedence are CSS's: name over class over element, and the later rule wins a tie.

**States.** `:hover` holds on the node under the pointer and its ancestors, `:active` on the node a
press started on (and its ancestors) until release, `:focus` on the last action node pressed until
a press lands elsewhere. A state change runs the cascade again and overwrites what code wrote to the
node's paint — text content survives. `:checked` holds on a toggle that is on, `:disabled` on a
`disabled` node.

**Control parts.** A toggle and a slider are drawn with nodes the engine adds under them, styled
like any other: `toggle .toggle-check` (paint it under `toggle:checked`), and `slider .slider-fill`,
`.slider-thumb`, `.slider-rest`. The fill and the rest split the track by the value; the default
base sheet under every document's own gives all of them a look, which the document's rules override.

**Variables.** `--name: value` in `:root`, read with `var(--name)`.

**Inherited.** `color`, `font-family` and `font-size` pass to children unless set there. A label with no `font-size` anywhere up its chain is 24.

### Properties

| Property | Values |
|---|---|
| `display` | `flex`, `grid`, `none` |
| `overflow` | `visible`, `hidden` (cuts its children at its padding box, rounded corners included), `scroll` (also scrolls, with a thumb) |
| `position` | `relative`, `absolute` |
| `left`, `top`, `right`, `bottom` | length |
| `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height` | length, `auto` |
| `flex-direction` | `row`, `column`, `row-reverse`, `column-reverse` |
| `flex-wrap` | `nowrap`, `wrap` |
| `flex-grow`, `flex-shrink` | number |
| `flex-basis` | length, `auto` |
| `grid-template-columns`, `grid-template-rows` | tracks: length, `auto`, `1fr`, `repeat(4, 1fr)`, `minmax(80px, 1fr)` |
| `align-items`, `align-self`, `align-content` | `flex-start`, `flex-end`, `center`, `stretch`, `baseline` |
| `justify-content` | `flex-start`, `flex-end`, `center`, `space-between`, `space-around`, `space-evenly` |
| `gap` | length, one or two. A `%` is of the element's own size, as in CSS: one with an `auto` size is laid out with no gap, which then overflows it |
| `margin`, `padding` | length, one to four, and the `-top`/`-right`/`-bottom`/`-left` longhands |
| `background-color`, `color` | `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, a CSS colour name |
| `border` | `<width> <color>` |
| `border-width`, `border-color` | length; colour |
| `border-radius` | length |
| `font-family` | `"Inter"`, `"Fira Code"`, or the family of a `.ttf`/`.otf` in the project |
| `font-size` | length |
| `background-image` | `url("button.png")` relative to the file writing it (the `.kcss`, or the `.kxml` for an inline `style`), PNG, JPEG or SVG; `none`. An engine image is written as if it sat in the project's `assets/`, where a build puts it: `url("../textures/prototype/dark/dark_texture_01.png")` from `assets/ui/` |
| `-kooch-mask` | `url("shape.png")`, as `background-image`; `none`. Its alpha cuts the node and its children |
| `-kooch-mask-mode` | `alpha`, or `luminance`: black cuts too, for an image with no transparency |
| `-kooch-slice` | length, one to four: image texels kept unstretched at each edge |
| `-kooch-tint` | colour that multiplies the image |
| `-kooch-material` | a path to a shader with `kind: ui`, relative to the file writing it, as `background-image` |
| `-kooch-param-<name>` | a value for what the material declares: one to four numbers, or a colour. The name in lower case |

**Images.** A `background-image` draws over the `background-color`, stretched to the node's box,
or 9-sliced when `-kooch-slice` is set: the corners keep their size (one canvas unit per texel) and
the edges and middle stretch. A state rule can swap it, which is how a button gets art per state:

```css
.button        { background-image: url("button.png"); -kooch-slice: 12px; }
.button:hover  { background-image: url("button_hover.png"); }
.button:active { background-image: url("button_press.png"); }
```

Saving the PNG shows the change without Play. A node's own image is not clipped to its
`border-radius`: to round a picture, put it inside a node with `overflow: hidden`.

**SVG.** An `.svg` is drawn at the size it shows on screen, so an icon stays sharp on a handheld
and on a 4K monitor alike. It is drawn once per size class (16, 32, 64… up to 2048 pixels on its
longer side), and each node uses the class just above its size, so the same icon at 24 and at 480
pixels is two drawings. A transform does not redraw it: to animate one, lay the node out at its
largest size and animate `scale` below 1. A
`-kooch-slice` on an SVG is in the file's own units. Text inside an SVG and images embedded in it
are not drawn: convert text to paths before exporting. As a material texture, an SVG is drawn once
at the size it declares.

With the `vello` feature on (`features = ["vello"]` on `kooch`; the editor always has it), SVGs are
drawn on the GPU by [Vello](https://github.com/linebender/vello) instead of on the CPU, into a
texture of their own. The size classes stay the same; what changes is that a new size or a changed
file costs a GPU pass instead of a CPU raster, and nothing is redrawn while the set on screen
stays the same. In a profile it is the `ui: vello` scope. Without the feature, nothing of Vello
is in the build.

**Masks.** `overflow: hidden` on a node with `border-radius` cuts its children to the rounded shape.
`-kooch-mask` cuts by the alpha of an image, stretched to the node's box, which is how a bar fill
follows a curve: the bar has the mask, and the fill grows inside it. Masks nest up to 16 deep, and
the pointer does not hit what a mask took away.

**Materials.** `-kooch-material` draws the node's box with a [`kind: ui` shader](shaders.md#ui)
instead of its background and image: one quad, so `-kooch-slice` does not apply, and the shader
reads the image with `sample_image`. Text and children draw on top as usual. The engine ships
two recipes, written from `assets/ui/` as if they sat in the project:

```css
.health { -kooch-material: "../shaders/ui/ring.wgsl"; -kooch-param-thickness: 0.2;
          background-color: #e0483a; border-color: #2a2a2a; }
```

A parameter follows a field with `bind:param.<name>`, or takes a value from code with no restyle:

```xml
<panel class="health" bind:param.fill="Health.ratio"/>
```

```rust
tree.set_param(node, "fill", 0.6);
```

A value set by code or a binding wins over the style until the document is rebuilt. `ring.wgsl`
fills an arc, `curved_bar.wgsl` bends a bar and `bezier_bar.wgsl` runs along a curve whose points you
drag in the UI Builder; copy any of them to start your own. A project's
[editor panel](../editor/panels.md) draws a material as a plain box: panels only redraw on change,
so `time` would stand still there.

`-kooch-curve: 0.1 0.5 0.5 0.1 0.9 0.5` gives a node a path through any number of points, as fractions
of its box, and everything inside it bends along it: images with their slices, text,
buttons and materials, with no shader of yours. The node itself stays straight. Its children lay out
as usual, then across the node becomes along the curve and the distance from its middle the distance
from the line, so a child centred and 40px tall is a band 40px thick. Clicks follow the bend, the
UI Builder drags the points, adds one with a double-click on the path and removes one with a
right-click, and `none` clears it. A trailing `closed` joins the ends.

A point is `x y` when it follows its neighbours, `x y / lx ly` with a smooth tangent of its own and
`x y | ax ay lx ly` with two independent ones, which is how a path gets a corner. Tangents are
fractions of the box too, relative to their point, so a circle in a wide node is an ellipse. The UI
Builder draws a grip per tangent, and dragging one on an automatic point makes it smooth, as on a 3D
spline; each point's mode is in the Inspector. The Inspector also starts a path from a line, arc,
circle or square: generated once, then yours to edit, and it replaces what was there. Circles are
four points, within 0.03 % of a true circle.

**Transforms.** `rotate: 15deg`, `scale: 1.2` (or `1.2 0.8`), `translate: 10px 4px` and
`transform-origin: 50% 100%` move a node and everything inside it after layout, so siblings stay
put: text, images, masks and clicks follow. `-kooch-rotate-x` and `-kooch-rotate-y` tilt it in 3D,
and `-kooch-perspective: 600px` is how far the eye is: smaller is stronger, 0 is flat. An angle
without a unit is in degrees. Inside a curve a transform moves the node before it bends; on the
curved node or above it, the bent result.

```css
.ring { -kooch-rotate-x: 60deg; -kooch-perspective: 500px; }
```

A system moves a node every frame with no relayout, the style's values being the start:

```rust
let mut tilt = tree.transform_of(card);
tilt.rotate_y = pointer.x * 20.0;
tree.set_transform(card, tilt);
```

`-kooch-flip: x`, `y` or `both` mirrors a node and everything inside it about its centre, after
layout: images, text, masks and clicks all follow, and siblings do not move. On a node with a curve
it mirrors the bent result, so `x` on a circle runs it counter-clockwise, fill included. Text reads
mirrored too, as it does in a mirror. What bends or mirrors is cut by the window where it lands, not
where it was laid out.

A band thicker than the curve's radius folds over
itself, a curve inside a curve replaces the outer one, and a clip or mask from above the node is read
as if it were straight.

`-kooch-fill-from` turns any node into a bar: `left`, `right`, `top`, `bottom` or `clock` show it and
what is inside it from that side, up to the param `fill`, so `-kooch-param-fill: 0.6` and
`bind:param.fill="Health.ratio"` drive it as they drive a material. `image` follows `-kooch-mask`:
the mask's alpha is the shape and its brightness the order, black first, so a grey ramp painted along
any path fills along it. A sliced image keeps its slices and one inside a `-kooch-curve` fills along
the curve. Put the empty bar and the frame in sibling nodes: the fill only cuts what it is on.

A bare `0` is a length. Colours are sRGB: `hsl()`, `lab()` and the rest are a warning.

Anything else is a warning naming the file and line, and the rule's other properties still apply.

## UI Builder

Its Hierarchy, Library, canvas and Inspector dock like the editor's panels: drag a divider to resize
them and a tab to move or stack them. The arrangement stays when you open another file.

Double-click a `.kxml` in the Asset Browser and it opens in the **UI Builder**: the document alone
on a canvas, drawn by the same pass that draws it in the game. No scene and no camera behind it.

The Inspector's style fields show what the element really gets. Bold is written on the element; grey
comes from the sheet, a parent or the engine, and hovering says which. Editing a grey field writes it
on the element.

- **Resolution** — the screen the canvas is laid out for: the handheld's 1280×800 first, then 720p,
  1080p, 1440p and a portrait phone. The layout is the game's: 1080 canvas units tall, the width
  from the aspect.
- **Sheet** — the `.kcss` the canvas wears. The markup never names one, so the Builder starts with
  the sheet a document in the open scene pairs it with, and the toolbar picks another.
- **Zoom** — the wheel zooms one step around the cursor, the middle button pans, and **F** or
  **Fit** frames the whole canvas. The canvas is rendered at the size it is shown, so text stays
  sharp up to 4096 pixels a side.
- **Rect gizmo** — the selected element's handles write its `width`/`height` to the inline style,
  in px. An element in the flow is placed by the layout, so it only grows from its right and
  bottom. A `position: absolute` one gets all eight handles, and dragging it moves it: each edge
  keeps the side it is pinned by (`left` or `right`, `top` or `bottom`), and an unpinned one gets
  `left`/`top` from where it sits. A `%` the drag touches becomes px. One drag is one undo step.

- **Hierarchy** — the file's elements as it nests them. Under a `<use>`, greyed, its template's
  nodes: select a named one and the Inspector overrides it on this instance only, written as a
  `<set node="…">` (an empty field reads the template). An unnamed node needs a `name` in its
  template first. Click a row, or the canvas, to select. Drag a row to move it: onto
  the middle of a `panel`, `button` or the root it goes inside, last; onto a row's top or bottom
  edge it goes before or after. The right-click menu renames (sets `name`; a double-click does
  too), duplicates (**Ctrl+D**) and deletes (**Del**).
- **Library** — under the hierarchy: the standard elements (`panel`, `label`, `button`, `toggle`,
  `slider`) and, under *Project*, every other `.kxml`, which lands as a `<use>`. Drag one onto the
  hierarchy the same way, or onto the canvas: it goes inside what holds children, after what does
  not. A double-click adds it to the selection. A `.kxml` dragged from the Asset Browser lands as
  a `<use>` too. What lands is selected, with a default to be seen: a label reads *Label*, a
  button holds one.
- **Inspector** — every field's name says what it does on hover (the ⓘ beside it), and one
  the element sets is bold and ends in `*`, and so does the style group holding it. The selected
  element's attributes as typed fields, from what each element reads:
  a checkbox for `disabled` or `checked`, a number for a slider's `min`/`max`/`step`, the
  editor's asset picker for a `<use>`'s `src` (search, or drop a `.kxml` from the Asset Browser),
  text for `bind:value`. A bound toggle or slider gets a **preview** row: the value the canvas
  shows, since the field it binds is the game's; it is never saved. A `<label>` gets one for its text:
  what code writes there in the game, to see a long string lay out; empty shows the file's. An `on:*` action is typed or picked from **▾**, which lists every action
  the project's `.kxml` files send and its code reads with `ui_actions(resources, "…")`; a warning
  mark means no system reads it, a typo or code still to write, and the copy button puts the
  `ui_actions` line on the clipboard. Classes are chips: click one to remove it, type in *+ class*
  or pick from **▾** the classes the sheet names; one no rule names shows in warning colour.
  `font-family`, `background-image` and `-kooch-material` are asset pickers: search, or drop the
  file from the Asset Browser; the engine's fonts are listed as `[engine]`.
  *Matching selectors* lists the sheet's rules that reach the element, strongest first, with
  their declarations and line, `:hover` and other states included. A label
  in **bold** is set in the file; **↺** removes it, and the element reads its default again.
  Attributes the element does not know are listed under *Other attributes*, where any can be
  added. Last, the **inline style**, grouped as Display, Position, Flex, Align, Size, Margin,
  Padding, Text, Background and Border: a length with its unit (`px`, `%`, `auto`), a colour
  picker, a list for keywords. A group in bold has something set; *Unset all* clears it. Setting
  one side of a `margin: 2px 4px` splits it into its four sides. A value a field cannot show, like
  a `calc()`, stays as text, and *CSS* below shows the whole attribute to edit by hand.

Edits stay in memory until **Save** (or **Ctrl+S** with the Builder focused) writes the `.kxml`;
the tab's title carries a `*` until then. **Ctrl+Z** and **Ctrl+Y** step back and forward through
them, the Builder's own history: typing in one field is one step, up to the next click.
The file keeps its comments, prefixes and attribute order, and is re-indented. Saved elsewhere while
the Builder has no edits of its own, the file is read again; a `.kcss` saved anywhere restyles the
canvas.
