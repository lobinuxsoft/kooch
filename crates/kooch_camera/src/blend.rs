//! Easing for transitions between virtual cameras — the engine's own tween (`kooch_ecs::tween`),
//! under the names the rig has always used.

pub use kooch_ecs::tween::{
    CURVE_CHOICES as BLEND_CURVE_CHOICES, CURVE_CUBIC, CURVE_EXPO, CURVE_LINEAR, CURVE_QUAD,
    CURVE_SINE, EASE_CHOICES as BLEND_EASE_CHOICES, EASE_IN, EASE_IN_OUT, EASE_OUT, eased,
};
