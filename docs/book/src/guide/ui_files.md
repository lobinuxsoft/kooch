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
(`name`, `class`, `on:*`) apply to the instance's root.

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
styled by the document's sheet, not by one of its own, and saving the template rebuilds every
document that uses it. Names inside an instance stay as written, so two instances both have a
`label`; reach one through the instance's own name.

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
input system. A `UiDocument` has five action fields; an empty one reads the engine's default:

| Field | Type | Default |
|---|---|---|
| **Point** | Vector2 | the mouse (`Pointer X`/`Pointer Y`) |
| **Click** | Button | left mouse button |
| **Navigate** | Vector2 | arrows, d-pad, left stick |
| **Submit** | Button | Enter, Space, South |
| **Cancel** | Button | Escape, East |

A press focuses the node with an action it lands on, and a press on the game drops focus. With
focus, **Navigate** moves it to the nearest focusable node that way — once when pushed, then
repeating after half a second — **Submit** clicks it, and **Cancel** sends the nearest `on:cancel`.
Only the document holding focus reads Navigate, Submit and Cancel, so a HUD nobody focused never
takes the stick; the game still reads them too, so a button that keeps focus answers the jump key.
`tabindex="-1"` is for that.

A system answers an action by name: `ui_actions(resources, "hud.count")` is every document that sent
it this frame, once per send.

> Live today: `on:click`, `on:focus`/`on:blur`, `on:cancel`, `on:change`, `bind:value` and pad
> navigation on Overlay documents.

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
is of the parent. Nothing else.

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
| `position` | `relative`, `absolute` |
| `left`, `top`, `right`, `bottom` | length |
| `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height` | length, `auto` |
| `flex-direction` | `row`, `column`, `row-reverse`, `column-reverse` |
| `flex-wrap` | `nowrap`, `wrap` |
| `flex-grow`, `flex-shrink` | number |
| `flex-basis` | length, `auto` |
| `align-items`, `align-self`, `align-content` | `flex-start`, `flex-end`, `center`, `stretch`, `baseline` |
| `justify-content` | `flex-start`, `flex-end`, `center`, `space-between`, `space-around`, `space-evenly` |
| `gap` | length, one or two |
| `margin`, `padding` | length, one to four, and the `-top`/`-right`/`-bottom`/`-left` longhands |
| `background-color`, `color` | `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, a CSS colour name |
| `border` | `<width> <color>` |
| `border-width`, `border-color` | length; colour |
| `border-radius` | length |
| `font-family` | `"Inter"`, `"Fira Code"`, or a font the project registers |
| `font-size` | length |
| `background-image` | `url("button.png")` relative to the `.kcss`, PNG or JPEG; `none` |
| `-kooch-slice` | length, one to four: image texels kept unstretched at each edge |
| `-kooch-tint` | colour that multiplies the image |
| `-kooch-material` | a path to a `.shader` with `kind: ui` (#1690) |

**Images.** A `background-image` draws over the `background-color`, stretched to the node's box,
or 9-sliced when `-kooch-slice` is set: the corners keep their size (one canvas unit per texel) and
the edges and middle stretch. A state rule can swap it, which is how a button gets art per state:

```css
.button        { background-image: url("button.png"); -kooch-slice: 12px; }
.button:hover  { background-image: url("button_hover.png"); }
.button:active { background-image: url("button_press.png"); }
```

Saving the PNG shows the change without Play. Images are not clipped to `border-radius`.

A bare `0` is a length. Colours are sRGB: `hsl()`, `lab()` and the rest are a warning.

Anything else is a warning naming the file and line, and the rule's other properties still apply.

## UI Builder

Double-click a `.kxml` in the Asset Browser and it opens in the **UI Builder**: the document alone
on a canvas, drawn by the same pass that draws it in the game. No scene and no camera behind it.

- **Resolution** — the screen the canvas is laid out for: the handheld's 1280×800 first, then 720p,
  1080p, 1440p and a portrait phone. The layout is the game's: 1080 canvas units tall, the width
  from the aspect.
- **Sheet** — the `.kcss` the canvas wears. The markup never names one, so the Builder starts with
  the sheet a document in the open scene pairs it with, and the toolbar picks another.
- **Zoom** — the wheel zooms one step around the cursor, the middle button pans, and **F** or
  **Fit** frames the whole canvas. The canvas is rendered at the size it is shown, so text stays
  sharp up to 4096 pixels a side.

- **Hierarchy** — the file's elements as it nests them; a `<use>` is one row, its template's
  inside stays the template's. Click a row, or the canvas, to select.
- **Inspector** — the selected element's attributes as typed fields, from what each element reads:
  a checkbox for `disabled` or `checked`, a number for a slider's `min`/`max`/`step`, the
  editor's asset picker for a `<use>`'s `src` (search, or drop a `.kxml` from the Asset Browser),
  text for `bind:value`. An `on:*` action is typed or picked from **▾**, which lists every action
  the project's `.kxml` files send and its code reads with `ui_actions(resources, "…")`; a warning
  mark means no system reads it, a typo or code still to write, and the copy button puts the
  `ui_actions` line on the clipboard. Classes are chips: click one to remove it, type in *+ class*
  or pick from **▾** the classes the sheet names; one no rule names shows in warning colour.
  *Matching selectors* lists the sheet's rules that reach the element, strongest first, with
  their declarations and line, `:hover` and other states included. A label
  in **bold** is set in the file; **↺** removes it, and the element reads its default again.
  Attributes the element does not know are listed under *Other attributes*, where any can be
  added. Last, the **inline style**, grouped as Display, Position, Flex, Align, Size, Margin,
  Padding, Text, Background and Border: a length with its unit (`px`, `%`, `auto`), a colour
  picker, a list for keywords. A group in bold has something set; *Unset all* clears it. Setting
  one side of a `margin: 2px 4px` splits it into its four sides. A value a field cannot show, like
  a `calc()`, stays as text, and *CSS* below shows the whole attribute to edit by hand.

Edits stay in memory until **Save** writes the `.kxml`; the tab's title carries a `*` until then.
The file keeps its comments, prefixes and attribute order, and is re-indented. Saved elsewhere while
the Builder has no edits of its own, the file is read again; a `.kcss` saved anywhere restyles the
canvas.
