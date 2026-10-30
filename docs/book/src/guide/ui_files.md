# UI files

> 🚧 The syntax is frozen (#1693); the parsers arrive with #280. Until then a document is built from
> code, as in [UI](./ui.md).

A `UiDocument` points at two files, split like Unity's UXML and USS:

| File | Says | Format |
|---|---|---|
| `.kooch_ui` | which nodes, nested how, what they do | XML |
| `.kooch_style` | how they look | a subset of CSS |

The document is built from them in whichever process shows it: the editor's own with Play stopped,
so you design without running the game, and the game's in Play.

## Markup

```xml
<ui style="menu.kooch_style">
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
| `toggle` | A checkbox; its value is a `bool`. |
| `slider` | A track with a thumb; its value is an `f32` between `min` and `max`. |
| `use` | Another `.kooch_ui`, instanced here. |

Every element takes `name` (what `tree.query` finds, unique in the document) and `class` (any
number, space separated).

### Templates

`use` instances a file. `<set node="…" attribute="…"/>` inside it overrides one attribute of a named
node of the instance — the same button file used twice with two texts. Attributes on `use` itself
(`name`, `class`, `on:*`) apply to the instance's root.

```xml
<use src="menu_button.kooch_ui" name="quit" class="danger" on:click="game.quit">
  <set node="label" text="Salir"/>
</use>
```

### Events and bindings

`on:<event>="action"` names an action, it does not point at a function: the file stays data, the
editor can list the actions on offer, and it works the same when the game runs in another process.

| Event | On | Carries |
|---|---|---|
| `on:click` | `button` | — |
| `on:change` | `toggle`, `slider` | the new value |
| `on:focus` / `on:blur` | anything focusable | — |

`bind:value="Type.field"` keeps a `toggle` or `slider` in step with a field of a resource or of a
component on the document's entity, both ways: moving the slider writes the field, and changing the
field moves the slider.

A system hears the UI the way it hears input, by reading events:

```rust
fn pause_menu(resources: &mut Resources) {
    for event in resources.events::<UiEvent>() {
        if event.action == "game.quit" {
            // ...
        }
    }
}
```

Or the project registers the action once, and the UI runs it:

```rust
app.add_ui_action("game.quit", |resources: &mut Resources, _event: &UiEvent| { /* ... */ });
```

> The API names above are the design; they land with #96's input phase and #280's bindings phase.

## Style

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

**Variables.** `--name: value` in `:root`, read with `var(--name)`.

**Inherited.** `color`, `font-family` and `font-size` pass to children unless set there.

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
| `material` | a path to a `.shader` with `kind: ui` (#1690) |

Anything else is a warning naming the file and line, and the rule's other properties still apply.
