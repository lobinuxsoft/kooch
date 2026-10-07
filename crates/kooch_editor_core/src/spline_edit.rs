//! Where a knot added to a [`Spline`] lands.
//!
//! 🔴 A reflected list's `element` is `T::default()` — one fixed value, shared by every `Vec<T>` in
//! the engine (#1201). So the Inspector's add button can only ever produce a knot at the origin,
//! and a path authored by adding points would stack them all on top of each other.
//!
//! This runs where the field edit is dispatched rather than where it is emitted, so it covers every
//! route a knot can arrive by — the Inspector, a multi-selection edit, anything later — and the
//! placement stays **one** action, so adding a point is one undo step and crosses the wire once.

use kooch_ecs::reflect::{ReflectValue, list_from, list_value};
use kooch_ecs::spline::{Knot, eval};

/// The list with a freshly added knot moved ahead of the one before it, or `None` when there is
/// nothing to place.
///
/// ⚠️ The signature of "freshly added" is a last knot that is **exactly** `Knot::default()`. A knot
/// an author deliberately left at the local origin with every field untouched would be moved once;
/// moving it back is one drag, and the alternative is every added point landing on the origin.
pub(crate) fn placed(value: &ReflectValue) -> Option<ReflectValue> {
    let mut knots: Vec<Knot> = list_from(value.clone(), None, "points").ok()?;
    // One knot has nothing to be ahead of, and the first one belongs at the origin it was added at.
    if knots.len() < 2 {
        return None;
    }
    let last = *knots.last()?;
    if last != Knot::default() {
        return None;
    }

    let previous = knots[knots.len() - 2];
    // 🔴 Read WITHOUT the new knot. An `Auto` tangent is derived from a knot's neighbours, and one
    // of this knot's neighbours is the new one sitting at the origin — behind it. Including it makes
    // the curve appear to double back and the added point lands the wrong way down the path.
    let settled = &knots[..knots.len() - 1];
    // Ahead means along the way the curve was already arriving, which is the direction it would
    // continue in. Local forward keeps the second point sensible, with no direction yet to continue.
    let heading = match settled.len() >= 2 {
        true => eval::tangents(settled, false, settled.len() - 2, settled.len() - 1).1,
        false => glam::Vec3::ZERO,
    }
    .normalize_or(glam::Vec3::NEG_Z);
    *knots.last_mut()? = Knot {
        position: previous.position + heading * spacing(&knots),
        ..last
    };
    Some(list_value(&knots))
}

/// How far ahead to place it: the step the author was already using, so a path keeps its rhythm.
/// `DEFAULT_STEP` with only one gap to judge by.
fn spacing(knots: &[Knot]) -> f32 {
    let step = match knots.len() >= 3 {
        true => knots[knots.len() - 2]
            .position
            .distance(knots[knots.len() - 3].position),
        false => DEFAULT_STEP,
    };
    // 🔴 `!(step > 0.0)`, not `step <= 0.0`: two coincident knots give zero, and a NaN would pass a
    // `<=` and place the new knot nowhere at all.
    match !(step > 0.0) || !step.is_finite() {
        true => DEFAULT_STEP,
        false => step,
    }
}

/// Where a knot lands with no spacing to copy. A metre reads at the scale a blockout is built at.
const DEFAULT_STEP: f32 = 1.0;

#[cfg(test)]
mod tests;
