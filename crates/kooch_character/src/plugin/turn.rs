//! Standing the body up and pointing it where it is steered.

use glam::{Mat3, Quat, Vec3};

/// The orientation a character wants: standing on `up`, looking along
/// `facing`.
///
/// `facing` is flattened against `up` first, so steering into a slope
/// turns the body along the slope rather than into it.
///
/// With nothing to steer by it keeps the way it is already looking. That
/// is not the same as keeping the pose: standing up is not optional, and
/// a character with no [`Facing`](crate::Facing) at all still has to
/// stand on the planet it is on.
pub fn wanted(up: Vec3, facing: Vec3, current: Quat) -> Quat {
    let Some(up) = up.try_normalize() else {
        return current;
    };
    // Lying flat looking at the sky leaves no yaw in the forward axis,
    // so the body's own up is the last thing left to keep.
    let Some(forward) = flattened(facing, up)
        .or_else(|| flattened(current * Vec3::NEG_Z, up))
        .or_else(|| flattened(current * Vec3::Y, up))
    else {
        return current;
    };
    let right = forward.cross(up);
    Quat::from_mat3(&Mat3::from_cols(right, up, -forward))
}

/// A direction with the part along `up` taken out, or `None` when
/// nothing is left of it.
fn flattened(direction: Vec3, up: Vec3) -> Option<Vec3> {
    (direction - up * direction.dot(up)).try_normalize()
}

/// The up a body leans on when it is accelerating.
///
/// A body pushed forward tips forward, and one braking tips back. The
/// angle that would exactly balance the push is `atan(a / g)`, and
/// `lean` is the fraction of it to take — `1` leans as far as the
/// acceleration could hold.
///
/// Tilting the up rather than pushing off-centre, because the
/// orientation is authored: an off-centre impulse would be erased the
/// same step by the turn that sets the pose.
///
/// Weightless is upright. `atan` of a divide by zero is a right angle,
/// which is a character lying flat in orbit.
pub fn leaned(up: Vec3, acceleration: Vec3, weight: f32, lean: f32) -> Vec3 {
    if weight <= 1e-4 || lean.abs() <= 1e-4 {
        return up;
    }
    let Some(up) = up.try_normalize() else {
        return up;
    };
    let across = acceleration - up * acceleration.dot(up);
    let Some(direction) = across.try_normalize() else {
        return up;
    };
    let (sin, cos) = ((across.length() / weight).atan() * lean).sin_cos();
    (up * cos + direction * sin).normalize_or(up)
}

/// How far to turn this step.
///
/// `speed * dt` clamped to one: the whole remaining turn, never past it,
/// so a large step or a slow frame cannot overshoot and wobble.
pub fn towards(current: Quat, target: Quat, speed: f32, dt: f32) -> Quat {
    let step = (speed * dt).clamp(0.0, 1.0);
    current.slerp(target, step).normalize()
}

#[cfg(test)]
mod tests;
