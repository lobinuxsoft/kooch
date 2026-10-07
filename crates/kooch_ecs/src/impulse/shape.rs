//! The four signal shapes, copied keyframe for keyframe from Cinemachine (#1255).
//!
//! 🔴 These numbers are not taste, they are the reason a bump feels like a bump. Read from
//! `CinemachineImpulseDefinition.cs`'s `s_StandardShapes` — Unity `AnimationCurve`s, so the
//! evaluation is cubic Hermite between keyframes and the tangents carry as much of the shape as the
//! values do. A linear reading of the same points is a different signal.
//!
//! Deterministic by construction: a curve, never a random walk, so two players feel the same hit.

/// A single recoil: full amplitude at once, falling away.
pub const RECOIL: u32 = 0;
/// A knock: down, up past zero, back.
pub const BUMP: u32 = 1;
/// A damped oscillation that starts on the back foot.
pub const EXPLOSION: u32 = 2;
/// A long roll of swells.
pub const RUMBLE: u32 = 3;

/// What the Inspector offers.
pub static SHAPE_CHOICES: &[crate::reflect::FieldChoice] = &[
    crate::reflect::FieldChoice {
        label: "Recoil",
        value: RECOIL as i64,
    },
    crate::reflect::FieldChoice {
        label: "Bump",
        value: BUMP as i64,
    },
    crate::reflect::FieldChoice {
        label: "Explosion",
        value: EXPLOSION as i64,
    },
    crate::reflect::FieldChoice {
        label: "Rumble",
        value: RUMBLE as i64,
    },
];

/// One keyframe: time, value, and the tangent on each side.
struct Key {
    t: f32,
    value: f32,
    tangent: f32,
}

const fn key(t: f32, value: f32, tangent: f32) -> Key {
    Key { t, value, tangent }
}

const RECOIL_KEYS: &[Key] = &[key(0.0, 1.0, -3.2), key(1.0, 0.0, 0.0)];

const BUMP_KEYS: &[Key] = &[
    key(0.0, 0.0, -4.9),
    key(0.2, 0.0, 8.25),
    key(1.0, 0.0, -0.25),
];

const EXPLOSION_KEYS: &[Key] = &[
    key(0.0, -1.4, -7.9),
    key(0.27, 0.78, 23.4),
    key(0.54, -0.12, 22.6),
    key(0.75, 0.042, 9.23),
    key(0.9, -0.02, 5.8),
    key(0.95, -0.006, -3.0),
    key(1.0, 0.0, 0.0),
];

const RUMBLE_KEYS: &[Key] = &[
    key(0.0, 0.0, 0.0),
    key(0.1, 0.25, 0.0),
    key(0.2, 0.0, 0.0),
    key(0.3, 0.75, 0.0),
    key(0.4, 0.0, 0.0),
    key(0.5, 1.0, 0.0),
    key(0.6, 0.0, 0.0),
    key(0.7, 0.75, 0.0),
    key(0.8, 0.0, 0.0),
    key(0.9, 0.25, 0.0),
    key(1.0, 0.0, 0.0),
];

fn keys(shape: u32) -> &'static [Key] {
    match shape {
        BUMP => BUMP_KEYS,
        EXPLOSION => EXPLOSION_KEYS,
        RUMBLE => RUMBLE_KEYS,
        _ => RECOIL_KEYS,
    }
}

/// The signal of `shape` at `t`, where `t` runs 0 to 1 across the impulse's duration.
pub fn at(shape: u32, t: f32) -> f32 {
    let keys = keys(shape);
    // Outside its window an impulse contributes nothing, which is what lets a finished one be
    // dropped rather than decayed forever. `!(t > 0.0)` catches NaN with the rest.
    if !(t > 0.0) {
        return keys[0].value;
    }
    if t >= 1.0 {
        return 0.0;
    }
    match keys.windows(2).find(|pair| t < pair[1].t) {
        Some(pair) => hermite(&pair[0], &pair[1], t),
        None => 0.0,
    }
}

/// Unity's `AnimationCurve` between two keyframes: cubic Hermite on the values and tangents.
///
/// 🔴 The tangents are scaled by the span, because a tangent is a slope in the curve's own units
/// and the basis functions work on a normalised one. Dropping that scale makes every shape with a
/// steep tangent — Bump's 8.25, Explosion's 23.4 — overshoot by the ratio.
fn hermite(from: &Key, to: &Key, t: f32) -> f32 {
    let span = to.t - from.t;
    if !(span > 0.0) {
        return to.value;
    }
    let u = (t - from.t) / span;
    let (u2, u3) = (u * u, u * u * u);
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    h00 * from.value + h10 * span * from.tangent + h01 * to.value + h11 * span * to.tangent
}

#[cfg(test)]
mod tests;
