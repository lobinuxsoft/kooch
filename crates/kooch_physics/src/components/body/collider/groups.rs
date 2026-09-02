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

/// The collision groups, named.
///
/// Sixteen of rapier's thirty-two bits, named generically because the
/// engine does not know what a project's layers mean. A game renames them
/// by shipping its own labels; what matters here is that the Inspector
/// shows *boxes* rather than a number, because a filtering mistake written
/// as an integer fails silently — two things pass through each other and
/// nothing says why.
///
/// The remaining sixteen are deliberately unnamed rather than absent: the
/// widget preserves bits it does not know about, so a project using the
/// high half by hand keeps it across an edit.
pub static GROUP_BITS: &[FieldChoice] = &[
    FieldChoice {
        label: "Group 1",
        value: 1 << 0,
    },
    FieldChoice {
        label: "Group 2",
        value: 1 << 1,
    },
    FieldChoice {
        label: "Group 3",
        value: 1 << 2,
    },
    FieldChoice {
        label: "Group 4",
        value: 1 << 3,
    },
    FieldChoice {
        label: "Group 5",
        value: 1 << 4,
    },
    FieldChoice {
        label: "Group 6",
        value: 1 << 5,
    },
    FieldChoice {
        label: "Group 7",
        value: 1 << 6,
    },
    FieldChoice {
        label: "Group 8",
        value: 1 << 7,
    },
    FieldChoice {
        label: "Group 9",
        value: 1 << 8,
    },
    FieldChoice {
        label: "Group 10",
        value: 1 << 9,
    },
    FieldChoice {
        label: "Group 11",
        value: 1 << 10,
    },
    FieldChoice {
        label: "Group 12",
        value: 1 << 11,
    },
    FieldChoice {
        label: "Group 13",
        value: 1 << 12,
    },
    FieldChoice {
        label: "Group 14",
        value: 1 << 13,
    },
    FieldChoice {
        label: "Group 15",
        value: 1 << 14,
    },
    FieldChoice {
        label: "Group 16",
        value: 1 << 15,
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
