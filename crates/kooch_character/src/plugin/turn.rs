//! Standing the body up and pointing it where it is steered.

use glam::{Mat3, Quat, Vec3};

/// The orientation a character wants: standing on `up`, looking along `facing` flattened against
/// it. With no steering it keeps its heading, but still stands up.
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

/// The up a body leans on while accelerating: `lean` times `atan(a / g)`, drawn into the pose since
/// the orientation is authored. Weightless is upright.
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

/// How far to turn this step: `speed * dt`, clamped to one so a slow frame cannot overshoot.
pub fn towards(current: Quat, target: Quat, speed: f32, dt: f32) -> Quat {
    let step = (speed * dt).clamp(0.0, 1.0);
    current.slerp(target, step).normalize()
}

#[cfg(test)]
mod tests;
