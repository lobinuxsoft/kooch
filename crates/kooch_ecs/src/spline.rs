//! [`Spline`] — the curve nothing in this engine had (#1261).
//!
//! 🔴 Evaluation is free functions over the points, never methods on the component. Five features
//! read the same curve — rails (#1262), a sweep (#1152), entities along it (#1428), gravity along it
//! (#1429), pipes and roads (#1092) — and a borrow of the storage is not a borrow of one component.

use glam::Vec3;

use crate::component::Component;
use crate::reflect::FieldRange;

#[allow(unused_imports)]
use crate::Reflect;

pub mod arc;
pub mod eval;
pub mod nearest;
pub mod ops;
pub mod placement;

/// A cubic curve through its points, in the entity's own space.
///
/// Authored in local space so moving the entity moves the whole path, and so a curve can be built
/// in a frame that spherical terrain transforms — a Bézier in flat world space runs through the
/// ground on a planet.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Spline")]
pub struct Spline {
    /// The points, first to last. Fewer than two is not a curve and evaluates to the first point.
    #[reflect(bare = "point")]
    pub points: Vec<Knot>,
    /// Closed joins the last point back to the first, so a circuit has no seam.
    pub closed: bool,
}

impl Default for Spline {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            closed: false,
        }
    }
}

impl Component for Spline {}

/// One point of a [`Spline`]: where it passes, how it leaves, and how the frame rolls there.
///
/// 🔴 AoS, against the SoA default, because evaluating a segment reads **one point and the next**
/// in full — position, both tangents and the roll together. Parallel arrays would turn one cache
/// line into four. The layout follows the access pattern, which is what the rule actually says.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct Knot {
    /// Where the curve passes, in the entity's space.
    pub position: Vec3,
    /// How the tangents are derived: one of the `TANGENT_*` constants.
    #[reflect(choices = TANGENT_CHOICES)]
    pub mode: u32,
    /// Arriving tangent, relative to `position`. Read only when `mode` is `TANGENT_BROKEN`.
    pub arriving: Vec3,
    /// Leaving tangent, relative to `position`. Read when `mode` is not `TANGENT_AUTO`.
    pub leaving: Vec3,
    /// Degrees the frame rolls about the tangent here, interpolated along the segment. What banks a
    /// road and what tilts a camera on a rail.
    #[reflect(range = ROLL_RANGE)]
    pub roll: f32,
    /// How thick the curve is here, in the entity's units, interpolated along the segment. What
    /// tapers a swept profile (#1152) and what thins a branch towards its tip.
    #[reflect(range = RADIUS_RANGE)]
    pub radius: f32,
}

impl Default for Knot {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            mode: TANGENT_AUTO,
            arriving: Vec3::ZERO,
            leaving: Vec3::ZERO,
            roll: 0.0,
            radius: DEFAULT_RADIUS,
        }
    }
}

impl Knot {
    /// A point at `position`, tangents derived.
    pub fn at(position: Vec3) -> Self {
        Self {
            position,
            ..Self::default()
        }
    }
}

/// A number that changes whenever the curve does.
///
/// 🔴 Every field that can bend the curve or thicken it, including the authored tangents. They are
/// ignored while a knot is `Auto` — `eval` derives both — but anything built from the curve has to
/// follow an `Aligned` or `Broken` knot whose grips were dragged.
///
/// Shared, so the two things built from a curve — a run of entities (#1428) and a swept mesh
/// (#1152) — cannot disagree about whether it moved.
pub fn curve_revision(points: &[Knot], closed: bool) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    closed.hash(&mut hasher);
    for knot in points {
        for value in [
            knot.position.x,
            knot.position.y,
            knot.position.z,
            knot.roll,
            knot.radius,
            knot.arriving.x,
            knot.arriving.y,
            knot.arriving.z,
            knot.leaving.x,
            knot.leaving.y,
            knot.leaving.z,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        knot.mode.hash(&mut hasher);
    }
    hasher.finish()
}

/// Tangents from the neighbours, Catmull-Rom. Nothing to author, and a moved point re-smooths the
/// two beside it.
pub const TANGENT_AUTO: u32 = 0;
/// `leaving` is authored and `arriving` mirrors it, so the curve stays smooth through the point.
pub const TANGENT_ALIGNED: u32 = 1;
/// Both authored. The only mode that can put a corner in the curve.
pub const TANGENT_BROKEN: u32 = 2;

pub static TANGENT_CHOICES: &[crate::reflect::FieldChoice] = &[
    crate::reflect::FieldChoice {
        label: "Auto",
        value: TANGENT_AUTO as i64,
    },
    crate::reflect::FieldChoice {
        label: "Aligned",
        value: TANGENT_ALIGNED as i64,
    },
    crate::reflect::FieldChoice {
        label: "Broken",
        value: TANGENT_BROKEN as i64,
    },
];

const ROLL_RANGE: FieldRange = FieldRange {
    min: -180.0,
    max: 180.0,
    step: 1.0,
};

/// Half a metre, so a new knot sweeps something a metre across — the scale of a path, a pipe or a
/// trunk, which is what a radius is authored for.
pub const DEFAULT_RADIUS: f32 = 0.5;

/// Never negative: a radius is a width, and a negative one turns a swept profile inside out.
pub const RADIUS_MIN: f32 = 0.0;
pub const RADIUS_MAX: f32 = 100.0;

/// Public because the gizmo and the viewport handle write this field too, and three copies of a
/// legal range are three chances for the Inspector's slider to disagree with a drag.
const RADIUS_RANGE: FieldRange = FieldRange {
    min: RADIUS_MIN as f64,
    max: RADIUS_MAX as f64,
    step: 0.05,
};

#[cfg(test)]
mod tests;
