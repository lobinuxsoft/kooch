# Input

A game never names a key. It points a component at an `.inputaction` asset, and the action decides
which controls feed it — keys, mouse buttons, gamepad buttons and axes, and **mouse motion**.
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

## Capturing the cursor

Mouse look wants the cursor held in place and hidden. Ask for it; the window applies it:

```rust
resources.insert(CursorMode::Captured); // held and hidden; motion keeps arriving
resources.insert(CursorMode::Free);     // visible again, free to leave the window
```

Absent means no opinion. Where the platform cannot lock the cursor in place (X11), it is confined
to the window instead — motion arrives either way, which is all mouse look reads.

### In the editor

While playing, **click the game image** to hand the cursor to the game. **`Esc`** takes it back, and
so does stopping or clicking another panel: the editor can always get its cursor back. The editor
applies this to its own window whatever process is running the game, so it works the same locally
and over a remote session.
