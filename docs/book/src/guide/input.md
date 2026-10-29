# Input

A game never names a key. It points a component at an `.inputaction` asset, and the action decides
which controls feed it — keys, mouse buttons, gamepad buttons and axes, **mouse motion** and the **mouse wheel**.
Actions are authored in the Input Map panel.

## Looking around: the mouse and the stick in one action

A camera that turns needs a `Vector2` action — call it `look` — with two composites:

| Composite | Right | Up | Processor |
|---|---|---|---|
| mouse | `Mouse / Motion X` | `Mouse / Motion Y` | `Scale` (see below) |
| stick | `Pad / RightStickX` | `Pad / RightStickY` | — |

Whichever is moving answers. Both read as the **same kind of number**, so a system consumes the
action with one formula whatever device the player holds:

```rust
let look = loaded.evaluate(input.look, backend).map(|v| v.vector2()).unwrap_or_default();
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
let zoom = loaded.evaluate(input.zoom, backend).map(|v| v.axis()).unwrap_or_default();
distance = (distance - zoom * metres_per_notch).clamp(min, max);
```

A trackpad reports pixels, which are converted at egui's 50 per line, so a notch and a swipe of
the same length move the camera alike. Each notch counts once, including in the editor's Play,
where the editor and the game do not share a frame rate.

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

While playing, **`Esc`** gives the cursor back without stopping, and a **click on the game image**
takes it again. Stopping or clicking another panel frees it too: the editor can always get its
cursor back. The editor
applies this to its own window whatever process is running the game, so it works the same locally
and over a remote session.

A game reads the same input under Play as in its build. The editor sends what it holds; the game
process turns that into one frame of `just_pressed` per press and drains the input events once per
frame, however fast each side runs.
