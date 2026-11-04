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

`on:click` running the project's code in the editor comes next (#279).
