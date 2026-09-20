//! Combine rules and collision groups — how two surfaces resolve, and
//! which pairs are considered at all.

use kooch_ecs::reflect::FieldChoice;

use crate::backend::CombineRule;

/// The mean of the two coefficients. Rapier's default.
pub const COMBINE_AVERAGE: u32 = 0;
/// The smaller value — the slipperier surface wins.
pub const COMBINE_MIN: u32 = 1;
/// The product — both surfaces have to be high.
pub const COMBINE_MULTIPLY: u32 = 2;
/// The larger value — the stickier surface wins.
pub const COMBINE_MAX: u32 = 3;
/// The sum, clamped.
pub const COMBINE_CLAMPED_SUM: u32 = 4;

/// Labels for the combine-rule dropdowns.
pub static COMBINE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Average",
        value: COMBINE_AVERAGE as i64,
    },
    FieldChoice {
        label: "Min (slipperier wins)",
        value: COMBINE_MIN as i64,
    },
    FieldChoice {
        label: "Multiply",
        value: COMBINE_MULTIPLY as i64,
    },
    FieldChoice {
        label: "Max (stickier wins)",
        value: COMBINE_MAX as i64,
    },
    FieldChoice {
        label: "Clamped sum",
        value: COMBINE_CLAMPED_SUM as i64,
    },
];


/// The backend rule for a discriminant, defaulting to the average for one
/// outside the known set — a scene from a newer editor stays loadable.
pub(super) fn combine_rule(discriminant: u32) -> CombineRule {
    match discriminant {
        COMBINE_MIN => CombineRule::Min,
        COMBINE_MULTIPLY => CombineRule::Multiply,
        COMBINE_MAX => CombineRule::Max,
        COMBINE_CLAMPED_SUM => CombineRule::ClampedSum,
        _ => CombineRule::Average,
    }
}
