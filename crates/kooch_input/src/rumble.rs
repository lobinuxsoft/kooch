//! Rumble (#57): both motors, for a while. Played by gilrs in a standalone game, sent back to the
//! editor in Play — the editor's process is the one holding the pad.

use serde::{Deserialize, Serialize};

/// One request: motor strengths in `[0, 1]` and how long. A new one replaces what the pad was
/// playing; all zero stops it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rumble {
    /// The low-frequency, heavy motor.
    pub strong: f32,
    /// The high-frequency, light motor.
    pub weak: f32,
    pub duration_ms: u32,
}

impl Rumble {
    pub fn is_stop(&self) -> bool {
        !(self.strong > 0.0 || self.weak > 0.0) || self.duration_ms == 0
    }
}

/// The host's name for a rumble it hands the editor, riding `Moved` replies.
pub const RUMBLE_SIGNAL: &str = "input.rumble";

/// A rumble on its way from the host to the pad the editor holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RumbleSignal {
    /// The pad as the editor's backend numbers it — the snapshot already speaks those ids.
    pub pad: u32,
    pub rumble: Rumble,
}

/// gilrs' effect for a request, on one pad. Kept alive by the caller: dropping it stops it.
pub(crate) fn effect(
    gilrs: &mut gilrs::Gilrs,
    pad: gilrs::GamepadId,
    rumble: Rumble,
) -> Result<gilrs::ff::Effect, gilrs::ff::Error> {
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Repeat, Replay, Ticks};

    let magnitude = |value: f32| (value.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
    let play_for = Ticks::from_ms(rumble.duration_ms);
    let scheduling = Replay {
        play_for,
        ..Default::default()
    };
    let motor = |kind| BaseEffect {
        kind,
        scheduling,
        ..Default::default()
    };
    EffectBuilder::new()
        .add_effect(motor(BaseEffectType::Strong {
            magnitude: magnitude(rumble.strong),
        }))
        .add_effect(motor(BaseEffectType::Weak {
            magnitude: magnitude(rumble.weak),
        }))
        .repeat(Repeat::For(play_for))
        .gamepads(&[pad])
        .finish(gilrs)
}
