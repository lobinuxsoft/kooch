# Screenshots

How the images in this book are produced, so they can be retaken consistently when the UI
changes.

## Procedure

```bash
cargo build -p kooch_editor          # once
./target/debug/kooch_editor          # launch; do NOT use KOOCH_EDITOR_AUTO_OPEN
```

Get the editor into the state you want, then, with its window focused:

```bash
spectacle -a -b -n -o /tmp/shot.png    # -a active window, -b background, -n no notification
magick /tmp/shot.png -crop WxH+X+Y +repage docs/book/src/images/<name>.png
```

The crop removes the compositor's drop shadow, which `-trim` does not, because the shadow is
not a uniform colour. Read the offsets off the raw capture once per screen resolution.

## Conventions

- **Dark theme**, which is the editor's default and what every existing image uses.
- **Crop to the window**, including its title bar. No desktop background.
- **Real data, not placeholders** — a scene with actual entities reads better than an empty
  one, and shows the panel doing its job.
- **PNG.** These are UI screenshots with flat colour and text; JPEG artefacts on 1px lines are
  worse than the file size saved.
- Name after what is shown, not where it goes: `hub.png`, `inspector-joint.png`.

## What is still missing

Only `hub.png` exists. The rest need someone to drive the UI, because the interesting states
are all several clicks past the launch screen:

- [ ] The full editor with a project open — the dock layout, all panels visible
- [ ] The World panel with a scene hierarchy
- [ ] The Inspector showing a component with several field kinds (drag values, a dropdown from
      `#[reflect(choices)]`, a checkbox row from `#[reflect(bits)]`)
- [ ] The Add Component menu, with a project's own component under its category
- [ ] The Console, filtered
- [ ] Play running, physics debug overlay on
