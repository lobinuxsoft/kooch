# Cameras

A game's camera is not moved by gameplay code. It is **directed**: you author virtual cameras —
entities that say where to stand and what to look at — and the engine picks one and copies its
pose onto the camera that renders. Switching shots is raising one virtual camera's priority; the
engine blends between them.

The design and the names come from Cinemachine, through phantom-camera (MIT). If you know either,
the components below will be familiar. Cameras need the `camera` feature.

## Three roles

| Component | On | Says |
|---|---|---|
| `CameraBrain` | The rendering camera (the one with `PerspectiveCamera`) | "Virtual cameras drive me" |
| `VirtualCamera` | Its own entity, one per shot | Priority, damping, which way is up |
| `CameraTarget` | Whatever is followed — the player | "Follow me", in a group |

🔴 **The brain is required.** A scene with virtual cameras and no enabled `CameraBrain` drives
nothing, and says so once in the log. With two rendering cameras — a base and an overlay — only
the one with the brain is moved.

The highest `priority` among the enabled virtual cameras drives the brain. Ties go to the lower
entity index, so the choice is stable.

### Targets are a group, not an entity

`VirtualCamera.group` names a number, and every `CameraTarget` with that `group` is followed —
their weighted centre when there are several (`weight`, default 1). A two-player camera is two
targets in group 0. The camera keeps no reference to an entity, so nothing breaks when a scene
reloads.

A virtual camera whose group is empty stays where it is.

## Body and aim: what the components do

A virtual camera alone moves nothing. What it does is the components **beside** it: one body
(where to stand) and one aim (where to look). The component you add is the mode; there is no
dropdown that has to agree with it.

**Bodies:**

| Component | Stands |
|---|---|
| `HardLockToTarget` | Exactly on the target — a first-person camera |
| `Follow { offset }` | At the target plus a fixed offset |
| `OrbitalFollow` | On a sphere around the target (`radius`), or on Cinemachine's three-ring FreeLook surface |
| `ThirdPersonFollow` | Over a shoulder: `shoulder_offset`, `vertical_arm_length`, `camera_side`, `camera_distance` |
| `PositionComposer` | Wherever puts the target at a chosen screen point |

**Aims:**

| Component | Looks |
|---|---|
| `HardLookAt` | Straight at the target |
| `RotateWithFollowTarget` | The way the target faces |
| `PanTilt` | Along the orbit's own `yaw` and `pitch` — a shoulder offset stays off centre |
| `RotationComposer` | At the target, held at a screen point with a dead zone and a soft zone |

A third-person camera is `OrbitalFollow` + `HardLookAt`; a shooter is `ThirdPersonFollow` +
`PanTilt`.

The orbit bodies take their angles from `VirtualCamera.yaw` and `pitch` (degrees; positive pitch
looks down), not from the target's rotation — a rolling ball's rotation is no direction to stand
in.

## Smoothing and up

- **`damping`** is seconds to settle, per world axis. Exact: a target that stops is reached in that
  time, not approached forever. Zero is rigid.
- **`rotation_damping`** is the same for turning.
- **`up_mode`** says which way is up: `UP_WORLD` (+Y), `UP_TARGET` (the target's own up), or
  `UP_GRAVITY` — away from the gravity where the target stands. `UP_GRAVITY` is what keeps the
  horizon level while you walk around a [planet](gravity.md).

## Turning it with the stick: `CameraOrbit`

`CameraOrbit` (in `kooch::kooch_camera`) turns a virtual camera's `yaw` and `pitch` from a rate.
Its `look` is written every frame, like a character's `Facing`, and `speed` is degrees per second
at a full stick. `pitch_min` and `pitch_max` bound it; `auto_recentre` eases the yaw back behind
the target when nobody is looking around.

To drive it from an input action with no code, add `OrbitInput` (in
`kooch::kooch_camera::orbit::input`) beside it and point `look` at a `Vector2` action. The
[Input](input.md) page shows how one action reads both the mouse and the right stick.

## Switching shots

Two ways, depending on who decides.

**From code.** Raise its `priority`, or switch the others off — `enabled: false` takes a camera out of the election:

```rust
# use kooch::prelude::*;
# #[derive(Default, Reflect)]
# pub struct BossShot;
# impl Component for BossShot {}
/// Cuts to the boss camera while `fighting`, and back when it ends.
fn boss_camera(resources: &mut Resources, fighting: bool) {
    Query::<(&BossShot, &mut VirtualCamera)>::new(resources).for_each(|(_, camera)| {
        camera.enabled = fighting;
    });
}
```

**While something holds: `CameraWhen`.** It adds `boost` to the camera's priority while `asked` is
true — a held aim button, a trigger volume, a state. It adds rather than overwrites, so letting go
restores the authored priority. `toggle` makes a press turn it on and another turn it off.
`WhenInput` (in `kooch::kooch_camera::when::input`) fills `asked` from a button action, so an
over-the-shoulder aim on the left trigger needs no code.

Either way, the **brain** blends: `blend_duration` (0.5 s by default), along `blend_curve` and
`blend_ease`. A blend belongs to the brain, not to either camera, because it is between two of
them.

## Extensions

Each is a component beside a `VirtualCamera`:

| Component | Does |
|---|---|
| `Deoccluder` | Keeps the camera out of walls: sweeps a sphere of `radius` from the target and stops at the first hit. Needs `physics` |
| `CameraLookahead` | Frames where the target is going, from its recent positions |
| `LensOverride` | Asks the lens for a field of view, near and far while this camera is live |
| `CameraOffset` | Shifts the final pose |
| `ImpulseListener` | Shakes on impulses — a character's `LandingImpulse`, an explosion |

Shakes are curves, not noise: two players hit by the same explosion feel the same shake.

## Seeing it in the editor

The rig runs **only in Play**, in the game's own process — the editor authors camera behaviour and
never runs it, so nothing it computes can end up saved in a scene. Press Play to see a shot.

Select a virtual camera during Play and the Game view marks where its target sits against where the
rig means to hold it; lengths the picture cannot show — an arm, an orbit radius, a deoccluder's
pull — are written as text. The Inspector warns when a body or an aim sits on an entity with no
`VirtualCamera` beside it.
