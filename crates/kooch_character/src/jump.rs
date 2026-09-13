//! [`Jump`] — leaving the ground, more than once, and off a wall.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Jumping as a launch speed, not an impulse: an impulse jumps a heavy character lower, while
/// `speed² / 2g` is a height a designer can aim at.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Jump {
    /// Written by gameplay on the frame the button goes down, and
    /// cleared here once it is spent.
    pub wanted: bool,
    /// Launch speed along the local up, in m/s. The height that buys is
    /// `speed² / 2g`.
    pub speed: f32,
    /// How many more jumps are allowed with nothing underneath — `0` single, `1` double — refilled
    /// on landing.
    pub air_jumps: u32,
    /// Coyote time: seconds after leaving a ledge a jump still counts, so a press at the lip is not
    /// eaten.
    pub coyote: f32,
    /// Jump buffer: seconds before landing a press still counts, firing when the ground arrives.
    pub buffer: f32,
}

impl Default for Jump {
    fn default() -> Self {
        Self {
            wanted: false,
            // 5 m/s is about 1.3 m under earth gravity.
            speed: 5.0,
            air_jumps: 1,
            coyote: 0.12,
            buffer: 0.12,
        }
    }
}

impl Component for Jump {}

/// Jumping off a wall — a separate component because it takes the button when a jump would be
/// refused and sends the character away, not up.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct WallJump {
    /// Speed away from the wall, in m/s. This is what carries the
    /// character across a gap.
    pub push: f32,
    /// Speed along the local up, in m/s.
    pub climb: f32,
    /// Whether it also refills the air jumps: off, a wall is a rest; on, wall chains climb forever.
    pub refills: bool,
}

impl Default for WallJump {
    fn default() -> Self {
        Self {
            push: 6.0,
            climb: 5.0,
            refills: true,
        }
    }
}

impl Component for WallJump {}
