//! A pivot that turns on its own axis, so whatever is parented to it
//! orbits.

use glam::{Quat, Vec3};

use crate::component::Component;
use crate::query::Query;
use crate::transform::Transform;

#[allow(unused_imports)]
use crate::Reflect;

/// Turns this entity around an axis of its own, forever.
///
/// # Why a pivot instead of moving the lights
///
/// Nothing here mentions a light, a radius or a centre. Lamps circling
/// a mesh are lamps PARENTED to a spinning empty and pushed out from
/// it — the radius is each lamp's own `position`, the centre is the
/// pivot's. Putting the orbit in the component instead would make
/// lights a special case and leave the next thing that wants to spin —
/// a platform, a turret, a ring of pickups — with nothing to use.
///
/// It works because a light reads its `GlobalTransform`, not its local
/// one, so a parented lamp follows its parent the way a mesh does.
///
/// # Why the engine ships this at all
///
/// It began as a measurement helper: a light that never moves is the one
/// case the shadow page cache handles for free, so a benchmark built
/// with static lights measures the cache rather than the shadows.
///
/// 🔴 It is not one. Gated behind `testing`, it was absent from every
/// exported build — and an unregistered component is DROPPED on load
/// rather than refused, so a game shipped with lights that orbited in
/// the editor and stood still in the build, silently. A component whose
/// removal changes what a player sees is engine content by definition.
/// See [`crate::testing`] for why the module still carries that name.
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Animation")]
pub struct Spin {
    /// Which way the pivot turns, in its parent's space. Normalised when
    /// used, so the Inspector can hold whatever is typed.
    pub axis: Vec3,
    /// Degrees per second. Negative turns the other way.
    ///
    /// Degrees rather than radians because this is a field a person sets
    /// in the Inspector, and nobody authors 0.785.
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
///
/// 🔴 The step is scaled by the frame's delta, which is the whole reason
/// this is not `rotation * fixed_step`. The same scene has been measured
/// at 11 FPS and at 72 on the same handheld; a per-frame step would have
/// spun the lights six times faster on the build that got faster, and
/// "the lights sped up" is not a bug anyone traces back to a shadow
/// pass.
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
