//! [`Jump`] — leaving the ground, more than once, and off a wall.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Jumping, as a launch speed rather than an impulse.
///
/// # Why a speed and not a force
///
/// `impulse` divided by mass is what decides the height, so an impulse
/// makes a heavy character jump lower for no reason a designer asked
/// for. `speed` is metres per second straight up, and `speed² / 2g` is
/// the height — a number that can be aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Jump {
    /// Written by gameplay on the frame the button goes down, and
    /// cleared here once it is spent.
    pub wanted: bool,
    /// Launch speed along the local up, in m/s. The height that buys is
    /// `speed² / 2g`.
    pub speed: f32,
    /// How many more jumps are allowed with nothing underneath.
    ///
    /// `0` is a single jump, `1` a double. Refilled the moment the
    /// character is standing again.
    pub air_jumps: u32,
    /// How long after walking off a ledge a jump still counts as a
    /// ground jump, in seconds.
    ///
    /// Nobody presses the button on the frame they meant to. Without
    /// this a jump taken at the lip of a platform is simply eaten, and
    /// it reads as the controls dropping inputs.
    pub coyote: f32,
    /// How long before landing a jump still counts, in seconds.
    ///
    /// The other half of the same forgiveness: pressed a moment early,
    /// it fires on the frame the ground arrives instead of being lost.
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

/// Jumping off a wall, for a character that has one to push against.
///
/// Separate from [`Jump`] because it is a different move: it takes the
/// button in a state where a jump would otherwise be refused, and it
/// sends the character *away* from something rather than up. A project
/// that does not want it does not add the component.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct WallJump {
    /// Speed away from the wall, in m/s. This is what carries the
    /// character across a gap.
    pub push: f32,
    /// Speed along the local up, in m/s.
    pub climb: f32,
    /// Whether it also refills the air jumps.
    ///
    /// Off, a wall is a place to rest; on, a wall chain is unlimited
    /// height. Both are games, so it is a switch rather than a rule.
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
