# Input

A game never names a key. It points a component at an `.inputaction` asset, and the action decides
which controls feed it — keys, mouse buttons, gamepad buttons and axes, **mouse motion** and the **mouse wheel**.
Actions are authored in the Input Map panel.

## Reading an action

`LoadedActions` reads every action once a frame, in `Stage::Input`, so `Update` asks it by the guid
a component holds — no backend, and an unset or missing action reads at rest:

```rust,ignore
let loaded = resources.get::<LoadedActions>()?;
let stick = loaded.vector2(input.movement);
let jump = loaded.just_pressed(input.jump); // down this frame, not the last: a held button fires once
let gas = loaded.axis(drive.forward);
```

`evaluate(guid, backend)` still reads the device right now, for code that runs before
`Stage::Input`.

## Action maps: switching a group off

An action's **Map** field, in the Input Map panel, names the group it belongs to — `Player`, `UI`,
`Vehicle`. Switching a map off makes every action in it read as released and at rest, wherever it
is read from:

```rust,ignore
if let Some(loaded) = resources.get_mut::<LoadedActions>() {
    loaded.set_map("Player", !menu_open);
}
```

A pause menu switches `Player` off, so a jump pressed under the menu is not waiting when it closes;
the UI's own actions sit in `UI` and keep answering. An action with no map is always on — the one
that opens and closes the pause belongs there. Stop switches every map back on.

## Looking around: the mouse and the stick in one action

A camera that turns needs a `Vector2` action — call it `look` — with two composites:

| Composite | Right | Up | Processor |
|---|---|---|---|
| mouse | `Mouse / Motion X` | `Mouse / Motion Y` | `Scale` (see below) |
| stick | `Pad / RightStickX` | `Pad / RightStickY` | — |

Whichever is moving answers. Both read as the **same kind of number**, so a system consumes the
action with one formula whatever device the player holds:

```rust
let look = loaded.vector2(input.look);
yaw   += look.x * turn_speed_degrees_per_second * dt;
pitch += look.y * turn_speed_degrees_per_second * dt;
```

That works because **mouse motion is a velocity** — pixels per second — the way a stick's deflection
is a rate. A per-frame delta would have to be consumed without `dt`, and a stick with it, and one
action could not serve both. It also survives the editor's wire, where the editor and the game do
not share a frame rate.

The `Scale` on the mouse composite maps pixels per second into a stick's unit. `0.001` reads a brisk
thousand pixels a second as a full stick; that is the mouse sensitivity, and it belongs to the
binding rather than to the camera.

Up is up for both: screen space counts down, and the engine flips the mouse's Y so that moving it up
reads the way pushing a stick up does.

Mouse motion is the device's own report, not two cursor positions subtracted, so a turn **never
stops at the edge of the window**.

## Zooming: the mouse wheel

`Mouse / Scroll Y` (and `Scroll X`, for a tilting wheel or a trackpad's sideways swipe) reads the
wheel in **lines this frame**, with up as away from the player. An `Axis` action — say `zoom` —
bound to it is consumed **without** `dt`, because a wheel has notches, not a speed:

```rust
let zoom = loaded.axis(input.zoom);
distance = (distance - zoom * metres_per_notch).clamp(min, max);
```

A trackpad reports pixels, which are converted at egui's 50 per line, so a notch and a swipe of
the same length move the camera alike. Each notch counts once, including in the editor's Play,
where the editor and the game do not share a frame rate.

## Triggers: how far, not whether

`Gamepad / LeftTrigger2` and `RightTrigger2` read their **travel**, `0` at rest to `1` fully pulled,
so an `Axis` action bound to one regulates — throttle, brake, torque — instead of switching. A
`Button` action on the same trigger still fires once it counts as pressed. Some pads report their
triggers as `LeftZ`/`RightZ` axes instead; bind those the same way.

## Rumble

```rust
if let Some(pad) = backend.active_gamepad() {
    backend.rumble(pad, Rumble { strong: 0.8, weak: 0.3, duration_ms: 150 });
}
```

`strong` is the heavy, low motor and `weak` the light, high one, both in `[0, 1]`. A new rumble
replaces the one the pad was playing, and all zero stops it. A pad without force feedback ignores
it. In the editor's Play the pad belongs to the editor's process, so the game's request travels back
to it with the next world pull — a frame late, the same feel.

## Capturing the cursor

Mouse look wants the cursor held in place and hidden. Ask for it; the window applies it:

```rust
resources.insert(CursorMode::Captured); // held and hidden; motion keeps arriving
resources.insert(CursorMode::Free);     // visible again, free to leave the window
```

Absent means no opinion. Where the platform cannot lock the cursor in place (X11), it is confined
to the window instead — motion arrives either way, which is all mouse look reads.

### In the editor

**`Ctrl+P`** plays and stops, from anywhere — the game having the keyboard included. Playing brings
the **Game** panel to the front with the keyboard and the cursor; stopping brings the **Edit View**
back. Only the edge moves panels: one you switch to mid-play stays where you put it.

While playing, the editor applies the game's `CursorMode` to its own window: when the game changes
it, and on each **click on the game image**. A click on the game's UI therefore keeps the cursor
free if the game asks for that, as in its build; a game that never asks gets captured by a click.
**`Esc`** gives the cursor back without stopping, and so do stopping and clicking another panel: the
editor can always get its cursor back.

A game reads the same input under Play as in its build. The editor sends what it holds; the game
process turns that into one frame of `just_pressed` per press and drains the input events once per
frame, however fast each side runs.
