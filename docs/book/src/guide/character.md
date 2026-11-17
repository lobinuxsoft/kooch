# Characters

A character in Kóoch is a **dynamic capsule that floats**. A spring holds it a little above the
ground instead of letting it rest there, and that one decision buys the rest: steps are climbed
because the spring lifts over them, slopes are walked because the spring follows the ground,
crates are pushed because the capsule is a real body, and any gravity works because "up" is read
where the character stands — a planet's surface included.

Characters need the `character` feature (and `physics`, which it brings).

## The pieces

Start with three components on one entity, and add abilities one component at a time.

| Component | Who writes it | What it is |
|---|---|---|
| `PhysicsBody` (Dynamic) + `Collider` (capsule) | You, once | The body. See [Physics](physics.md) |
| `CharacterController` | You, once | How the capsule floats |
| `Walk` | You, once | How it gets up to speed and stops |
| `Facing` | **Your system, every frame** | Where it is being steered |
| `Grounded` | The engine | What is underfoot |
| `Touching` | The engine, if you add it | The wall ahead |

Abilities, each its own component: `CharacterJump`, `WallJump`, `Sprint`, `WallSlide`, `WallRun`
and `LandingImpulse`. A character without `WallRun` simply does not wall-run; there is no flag
to forget.

`CharacterJump` is the prelude's name for `kooch_character::Jump` — renamed there so it cannot
collide with a project's own `Jump`.

## Floating: `CharacterController`

The defaults are tuned for a capsule of radius 0.4 and half-height 0.5, which reaches 0.9 below its
origin.

| Field | Default | What it sets |
|---|---|---|
| `ride_height` | 1.1 | How high the **origin** rides. Must exceed the collider's reach below it (`r + h` for a capsule), or the capsule rests on the floor |
| `probe` | 1.8 | How far down to look for ground. Past `ride_height`, how far it can drop before it is falling |
| `probe_radius` | 0.35 | The probe is a swept sphere; much smaller finds the gaps between floor tiles |
| `stiffness` | 90 | How hard the spring pulls back to `ride_height` |
| `damping` | 7 | The landing dial. Critical is `2·sqrt(stiffness)`; a third of it dips once and recovers |
| `turn_speed` | 10 | How fast it turns to stand up and face its steering, in turns per second |
| `max_slope` | 50° | Steeper is a wall, not ground |
| `step_height` | 0.5 | Tallest thing that counts as a step |
| `reach` | 0.7 | How far ahead to look for a wall, for `Touching` |

**A bigger capsule needs a bigger `ride_height`.** If the character sits on the floor and will not
climb a step, the gap is gone: the capsule's bottom is touching, and the spring has nothing to
push against.

## Steering: `Facing`

`Facing.direction` is the one thing gameplay writes. Its direction turns the body; its length,
clamped to 1, is the throttle on `Walk.max_speed`.

🔴 **Write it every frame, zero included.** The engine does not clear it. A system that skips the
write when the stick is centred leaves the character walking on its own.

```rust
# use kooch::prelude::*;
#[derive(Default, Reflect)]
pub struct PlayerInput {
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub movement: Option<Guid>,
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub jump: Option<Guid>,
}
impl Component for PlayerInput {}

/// Steers and jumps every player from its input actions.
#[system]
pub fn steer_player(resources: &mut Resources) {
    let Some(loaded) = resources.get::<LoadedActions>() else {
        return;
    };
    Query::<(&PlayerInput, &mut Facing, Option<&mut CharacterJump>)>::new(resources).for_each(
        |(input, facing, jump)| {
            let stick = loaded.vector2(input.movement);
            // Stick up is forward, which is -Z.
            facing.direction = Vec3::new(stick.x, 0.0, -stick.y);
            if let Some(jump) = jump {
                jump.wanted |= loaded.just_pressed(input.jump);
            }
        },
    );
}
```

That steers in world space. A third-person game turns the stick by the camera's yaw first, so
"up" on the stick walks away from the camera.

`Facing` is flattened against the local up, so on a planet the same code walks along the surface.

## Walking: `Walk`

Walking is a **goal velocity**, not a push. A floating capsule has no friction to stop it, so
stopping has to be the same term as starting: the controller reaches for `direction × max_speed`
and lets go of whatever it had.

| Field | Default | What it sets |
|---|---|---|
| `max_speed` | 6 m/s | Top speed across the ground |
| `acceleration` | 60 m/s² | How fast the goal itself may change — how long it takes to agree with the stick |
| `max_force` | 90 m/s² | Ceiling on what is used to reach the goal — responsive, without shoving heavy crates |
| `air_control` | 0.3 | How much of that applies in the air. 0 is an unsteerable jump |
| `lean` | 0.35 | How far the body tilts into its acceleration |

`Sprint` multiplies `max_speed` by `speed` and `acceleration` by `eagerness` while it runs. Write
`wanted` every frame like `Facing`; with `toggle` on, a press starts it and stopping ends it.

## Jumping

`CharacterJump` is a **launch speed**, not an impulse: an impulse jumps a heavy character lower,
while a speed is a height — `speed² / 2g` — that a designer can aim at.

| Field | Default | What it sets |
|---|---|---|
| `wanted` | — | Set it on the frame the button goes down. The engine clears it once spent |
| `speed` | 5 m/s | About 1.3 m under Earth gravity |
| `air_jumps` | 1 | Extra jumps with nothing underneath. 0 is a single jump |
| `coyote` | 0.12 s | A press just after leaving a ledge still counts |
| `buffer` | 0.12 s | A press just before landing fires when the ground arrives |

Set `wanted` with `|=`, as above, and don't clear it yourself. The jump runs on the fixed physics
step; a frame can fall between two steps, and a flag cleared every frame would sometimes be gone
before any step read it.

`WallJump` takes the button when a jump would be refused — no ground, no air jumps left — and
there is a wall in `Touching`. It sends the character `push` away from the wall and `climb` up.
`refills` decides whether a wall gives the air jumps back: on, two walls facing each other can be
climbed forever.

## Walls

`WallSlide` and `WallRun` both read `Touching`, so add it to the character.

- **`WallSlide`** caps the fall at `max_fall` while the character steers into a wall harder than
  `grip`. A cap, not friction, so the slide is the same from any height.
- **`WallRun`** starts when the character arrives with at least `entry` m/s **along** the wall,
  and lasts `duration` seconds — refilled only by standing. `hold` is how much gravity it holds
  off; below 1, the sag tells the player the run is ending.

## Reading what happened

`Grounded` is written once per step for everything that asks — jumping, animation, footsteps, fall
damage — so they cannot disagree.

- `standing` — ground was found, no steeper than `max_slope`.
- `normal` — the slope underfoot, or zero for nothing.
- `distance` — the gap to it. **Never zero while standing**: the spring holds it open.

That last point is why **a landing is not a collision**. The capsule never touches the floor, so
no `CollisionStarted` and no `ContactForce` arrive. A landing is `standing` turning true.
`LandingImpulse` already turns that edge into a camera shake scaled by how fast the character was
falling; read `Grounded` yourself for a sound or a dust puff.

## While you tune

`Grounded`, `Touching`, `CharacterJump`, `Sprint` and `LandingImpulse` stream from the running game
to the editor's Inspector. Select the character during Play and watch `standing` and `distance`:
whether the spring is holding is a number there, not a guess.
