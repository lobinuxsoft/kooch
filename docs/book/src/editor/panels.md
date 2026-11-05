# Editor panels

A project adds its own panels to the editor the way it builds its game's UI: a `.kxml` and a
`.kcss`. No editor code, no plugin: the panel is a document.

Every `.kxml` under the project's `assets/editor/` is a panel. Its sheet is the `.kcss` with the
same name beside it. **Window → Project panels** lists them; opening one docks it like any other
panel, and the editor keeps it there between sessions. A panel whose file is deleted is closed.

```text
assets/editor/ball_tuning.kxml
assets/editor/ball_tuning.kcss
```

`assets/editor/` is the editor's alone, like Unity's `Editor` folder: a build leaves it out of the
game.

## Bound to the selection

In a panel, `bind:value="Type.field"` edits the **entity selected in the editor**, as the
Inspector does. A slider or a toggle moves the field; the field moves the control when anything
else changes it. With nothing selected, or a selection without that component, the control is
disabled.

```xml
<ui xmlns:bind="urn:kooch:bind">
  <panel class="rows">
    <label>Torque</label>
    <slider min="0" max="200" bind:value="RollDrive.torque" />
  </panel>
</ui>
```

An edit is an editor edit: **Ctrl+Z** undoes it, a whole drag at once, and it reaches the game in
the editor's Play like an Inspector edit does.

Saving the `.kxml` or the `.kcss` updates the open panel.

## Running your code

`on:click` in a panel runs a function of yours, in the editor. Put it under `src/tools/`, the code
twin of `assets/editor/`: a game does not compile that folder, so it can name what a game does not
have. Any `pub fn` there taking `&mut EditorCall` is an action, named after the function:

```rust
// src/tools/ball.rs
use kooch::kooch_plugin_api::EditorCall;

pub fn reset_tuning(call: &mut EditorCall) {
    for ball in call.selected() {
        call.set_number(ball, "RollDrive.max_torque", 6.0);
    }
}
```

```xml
<ui xmlns:on="urn:kooch:on">
  <button on:click="reset_tuning"><label>Reset</label></button>
</ui>
```

The editor picks the function up on its next build of the project; a new one needs that build
before the button answers. The signature has to fit on one line, or the scan misses it.

`EditorCall` is what the action can see and ask for:

| Call | What it does |
|---|---|
| `selected()` | The selected entities. |
| `number(entity, "Type.field")` | A number or bool field of a selected entity, a bool as 0 or 1. |
| `set_number(entity, "Type.field", value)` | Writes that field. **Ctrl+Z** undoes it, and it reaches the game. |
| `write_asset("folder/file.kcss", text)` | Writes a text file under `assets/`, replacing what is there. |

An action reads the editor's copy of the selection and asks for changes; the editor applies them
when it returns, so `number` does not see a `set_number` from the same call. A panic in an action
is logged and the editor keeps going.

A file only joins the Asset Browser with an extension the engine knows, or one your
[data assets](./data_assets.md) declare.
