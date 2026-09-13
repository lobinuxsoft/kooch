//! A pivot that turns on its own axis, so whatever is parented to it
//! orbits.

use glam::{Quat, Vec3};

use crate::component::Component;
use crate::query::Query;
use crate::transform::Transform;

#[allow(unused_imports)]
use crate::Reflect;

/// Turns this entity around an axis of its own, forever.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Animation")]
pub struct Spin {
    /// Which way the pivot turns, in its parent's space. Normalised when
    /// used, so the Inspector can hold whatever is typed.
    pub axis: Vec3,
    /// Degrees per second. Negative turns the other way.
    pub degrees: f32,
}

impl Component for Spin {}

// Not derived: a zeroed axis and a zeroed rate are a pivot that does
// nothing, which looks exactly like the component not being registered.
impl Default for Spin {
    fn default() -> Self {
        Self {
            axis: Vec3::Y,
            // A full turn every twelve seconds — slow enough to watch a
            // shadow sweep across a face, fast enough to see it move.
            degrees: 30.0,
        }
    }
}

/// Advances every pivot by one frame's worth of rotation.
pub fn spin_pivots(resources: &mut kooch_core::resource::Resources) {
    // Copied out rather than held: the query below borrows the component
    // storages, and a live borrow of `Time` would overlap it.
    let Some(delta) = resources
        .get::<kooch_core::time::Time>()
        .map(|time| time.delta_secs())
    else {
        return;
    };
    // A paused frame is not a reason to do the quaternion work, and a
    // negative delta is not a thing that should ever arrive.
    if delta <= 0.0 {
        return;
    }

    Query::<(&Spin, &mut Transform)>::new(resources).for_each(|(spin, transform)| {
        let axis = spin.axis.normalize_or_zero();
        if axis == Vec3::ZERO {
            return;
        }
        let step = Quat::from_axis_angle(axis, spin.degrees.to_radians() * delta);
        transform.rotation = (step * transform.rotation).normalize();
    });
}
