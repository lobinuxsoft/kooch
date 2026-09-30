//! [`OrbitalRings`] — the FreeLook surface: three horizontal circles joined by a spline, and the
//! pitch picks a point on it rather than swinging an arm (#1389).
//!
//! 🔴 The difference from a sphere is the whole feature. On a sphere the camera is always the same
//! distance out; on the rings it comes in and rises as you look down at a character, and pulls out
//! and drops as you look up. Cinemachine's `OrbitStyles.ThreeRing`, and what every third-person game
//! has one of.
//!
//! What is **not** in the placement is as telling: no rotation by pitch.
//! `pos = AngleAxis(horizontal, up) * SplineValue(verticalNormalized)` — the vertical axis is an
//! index into the surface, not an angle.

use glam::{Vec2, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;

/// The surface an [`Orbital Follow`](crate::FOLLOW_ORBITAL) vcam rides when its `orbit_style` asks
/// for it. Beside a [`VirtualCamera`](crate::VirtualCamera); on any other body it is inert.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct OrbitalRings {
    /// The ring reached at the top of the pitch's range.
    #[reflect(range = SIZE_RANGE)]
    pub top_radius: f32,
    #[reflect(range = HEIGHT_RANGE)]
    pub top_height: f32,
    /// The ring at the middle of it.
    #[reflect(range = SIZE_RANGE)]
    pub center_radius: f32,
    #[reflect(range = HEIGHT_RANGE)]
    pub center_height: f32,
    /// The ring reached at the bottom.
    #[reflect(range = SIZE_RANGE)]
    pub bottom_radius: f32,
    #[reflect(range = HEIGHT_RANGE)]
    pub bottom_height: f32,
    /// How taut the surface is past the top and bottom rings: the phantom knots beyond them are
    /// pulled this far towards the target, which is what loosens or tightens the ends.
    #[reflect(range = CURVATURE_RANGE)]
    pub spline_curvature: f32,
}

const SIZE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 100.0,
    step: 0.1,
};

const HEIGHT_RANGE: FieldRange = FieldRange {
    min: -100.0,
    max: 100.0,
    step: 0.1,
};

const CURVATURE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

impl Default for OrbitalRings {
    /// Cinemachine's own, so a rig authored against its documentation lands where it says.
    fn default() -> Self {
        Self {
            top_radius: 2.0,
            top_height: 5.0,
            center_radius: 4.0,
            center_height: 2.25,
            bottom_radius: 2.5,
            bottom_height: 0.1,
            spline_curvature: 0.5,
        }
    }
}

impl Component for OrbitalRings {}

/// The rings on `vcam`, if it has any. The only place that question is asked.
pub fn of(registry: &ComponentRegistry, vcam: Entity) -> Option<OrbitalRings> {
    registry.get_cpu::<OrbitalRings>()?.get(vcam).copied()
}

/// A knot on the surface: how high above the target, and how far out.
///
/// `x` is the height and `y` the radius, kept as a [`Vec2`] because the spline is solved on both at
/// once and a pair of scalars would be two solvers.
type Knot = Vec2;

impl OrbitalRings {
    /// Where the camera stands, relative to the target, at `t` — `0` the bottom ring, `0.5` the
    /// centre, `1` the top. The point is on the plane `up` is normal to, `radius` out along `back`
    /// and `height` along `up`.
    pub fn at(&self, t: f32, back: Vec3, up: Vec3) -> Vec3 {
        let knot = self.spline(t.clamp(0.0, 1.0));
        back * knot.y + up * knot.x
    }

    /// The five knots the surface is built from: the three rings, and a phantom beyond each end
    /// pulled towards the target by `spline_curvature`.
    fn knots(&self) -> [Knot; 5] {
        let bottom = Knot::new(self.bottom_height, self.bottom_radius);
        let centre = Knot::new(self.center_height, self.center_radius);
        let top = Knot::new(self.top_height, self.top_radius);
        let taut = self.spline_curvature.clamp(0.0, 1.0);
        [
            (bottom + (bottom - centre) * 0.5).lerp(Knot::ZERO, taut),
            bottom,
            centre,
            top,
            (top + (top - centre) * 0.5).lerp(Knot::ZERO, taut),
        ]
    }

    /// The point at `t` on the curve through the rings, `0` to `1` over the middle two segments —
    /// the phantom knots shape the ends and are never reached.
    fn spline(&self, t: f32) -> Knot {
        let knots = self.knots();
        let (first, second) = smoothed(&knots);
        // The first half of the range walks bottom → centre, the second centre → top.
        let (segment, t) = match t > 0.5 {
            true => (2, (t - 0.5) * 2.0),
            false => (1, t * 2.0),
        };
        bezier(
            t,
            knots[segment],
            first[segment],
            second[segment],
            knots[segment + 1],
        )
    }
}

/// Control points giving the curve through `knots` second-order smoothness, solved per axis with the
/// Thomas algorithm — Cinemachine's `ComputeSmoothControlPoints`.
///
/// 🔴 No cache. Cinemachine keeps one because it solves four axes into freshly allocated arrays;
/// this is two axes over five knots on the stack, once per vcam per frame.
fn smoothed(knots: &[Knot; 5]) -> ([Knot; 5], [Knot; 5]) {
    let mut first = [Knot::ZERO; 5];
    let mut second = [Knot::ZERO; 5];
    for axis in 0..2 {
        let of = |knot: Knot| match axis {
            0 => knot.x,
            _ => knot.y,
        };
        let n = knots.len() - 1;
        let (mut b, mut r) = ([0.0_f32; 5], [0.0_f32; 5]);
        let (mut a, mut c) = ([0.0_f32; 5], [0.0_f32; 5]);

        // Linear into the first segment, linear out of the last, and C² between.
        (a[0], b[0], c[0]) = (0.0, 2.0, 1.0);
        r[0] = of(knots[0]) + 2.0 * of(knots[1]);
        for i in 1..n - 1 {
            (a[i], b[i], c[i]) = (1.0, 4.0, 1.0);
            r[i] = 4.0 * of(knots[i]) + 2.0 * of(knots[i + 1]);
        }
        (a[n - 1], b[n - 1], c[n - 1]) = (2.0, 7.0, 0.0);
        r[n - 1] = 8.0 * of(knots[n - 1]) + of(knots[n]);

        for i in 1..n {
            let m = a[i] / b[i - 1];
            b[i] -= m * c[i - 1];
            r[i] -= m * r[i - 1];
        }

        let mut solved = [0.0_f32; 5];
        solved[n - 1] = r[n - 1] / b[n - 1];
        for i in (0..n - 1).rev() {
            solved[i] = (r[i] - c[i] * solved[i + 1]) / b[i];
        }
        for i in 0..n {
            let (one, two) = (solved[i], 2.0 * of(knots[i + 1]) - solved[i + 1]);
            match axis {
                0 => (first[i].x, second[i].x) = (one, two),
                _ => (first[i].y, second[i].y) = (one, two),
            }
        }
        let last = 0.5 * (of(knots[n]) + solved[n - 1]);
        match axis {
            0 => second[n - 1].x = last,
            _ => second[n - 1].y = last,
        }
    }
    (first, second)
}

/// A cubic Bézier at `t`.
fn bezier(t: f32, p0: Knot, p1: Knot, p2: Knot, p3: Knot) -> Knot {
    let t = t.clamp(0.0, 1.0);
    let d = 1.0 - t;
    p0 * (d * d * d) + p1 * (3.0 * d * d * t) + p2 * (3.0 * d * t * t) + p3 * (t * t * t)
}

#[cfg(test)]
mod tests;
