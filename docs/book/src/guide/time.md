# Time

Every system reads time from one resource, `Time`, through `Time::of(resources)`. It answers even
in a test or a tool that never added one, so no system invents its own fallback.

## Two clocks

| Call | Clock | Use it for |
|---|---|---|
| `delta_secs()` | **Game time** since the last frame: real time × the scale | Anything gameplay — movement, cooldowns, timers |
| `elapsed_secs()` | Game time since the start. Stands still while paused | A day/night cycle, a wave timer |
| `unscaled_delta_secs()` | **Real** time since the last frame | What a pause must not stop: menus, UI animation |
| `unscaled_elapsed()` | Real time since the start | Wall-clock measurements |

Picking the wrong one is the classic pause bug: a menu animated with `delta_secs` freezes with the
game it is pausing.

## Pause and slow motion

`set_scale` sets how fast game time runs: 0 pauses, 0.5 is slow motion, 1 is real time. Negative
or NaN reads as 0.

```rust
# use kooch::prelude::*;
/// Stops game time while a menu is open.
fn pause(resources: &mut Resources, menu_open: bool) {
    if let Some(time) = resources.get_mut::<Time>() {
        time.set_scale(if menu_open { 0.0 } else { 1.0 });
    }
}
```

Scale reaches everything that reads game time, physics included — a paused game takes no physics
steps. Input does not pause with it; switch the player's action map off as well, or a jump pressed
under the menu is waiting when it closes ([Input](input.md#action-maps-switching-a-group-off)).

**Stop resets the scale to 1.** A game stopped while paused would otherwise start the next Play
frozen.

## The fixed step

Physics, and anything else in the `Physics` and `PostPhysics` stages, runs on a **fixed step**:
60 Hz by default, `fixed_delta_secs()` long. Each frame adds its game time to an accumulator and the
engine runs as many whole steps as fit — none on a fast frame, two on a slow one — so a simulation
behaves the same at 30 fps and at 144.

- `Time::set_fixed_hz(hz)` changes the rate.
- The accumulator is capped at 250 ms. After a long hitch the game drops the time rather than
  running dozens of steps to catch up, which would make the next frame slower still.
- `render_alpha()` is how far the frame sits between the last step and the next. The engine already
  draws physics bodies interpolated with it, so a camera following one does not stutter; use it for
  anything of your own that moves on the fixed step.

A system in `Update` runs once per frame and reads `delta_secs`. One that pushes physics bodies is
still fine there: an impulse waits for the next step.

## Easing

For a value that moves from one number to another over a time, `kooch::kooch_ecs::tween::eased`
maps progress `t` (0 to 1) through a curve — `CURVE_LINEAR`, `CURVE_SINE`, `CURVE_QUAD`,
`CURVE_CUBIC`, `CURVE_EXPO` — with `EASE_IN`, `EASE_OUT` or `EASE_IN_OUT`. A component that tweens
stores the two as `u32` fields with `#[reflect(choices = CURVE_CHOICES)]` and
`#[reflect(choices = EASE_CHOICES)]`, and the Inspector shows them as dropdowns. The camera brain's
blends use the same curves.
