//! Turning rapier's debug walk into engine line segments.
//!
//! Rapier renders nothing. `DebugRenderPipeline` walks the world and calls
//! [`DebugRenderBackend::draw_line`] with a pair of world-space points and
//! a colour; the one required method is that. Everything curved arrives
//! already tessellated, so a sphere is segments rather than a centre and a
//! radius, and the pipeline's own default methods decompose polylines and
//! arcs down to `draw_line` for us.
//!
//! So this file is an adapter and a colour conversion, which is the whole
//! reason the issue said wiring it up was most of the work.
//!
//! # Compiled only for tools
//!
//! Behind the `debug-render` cargo feature, which also switches on
//! rapier's. A shipped game never enables it, so none of this reaches the
//! binary — #558's rule, applied where it is cheapest to apply.

use glam::Vec3;

use rapier3d::prelude::{
    DebugColor, DebugRenderBackend, DebugRenderMode, DebugRenderObject, DebugRenderPipeline,
    DebugRenderStyle,
};

use crate::backend::{DebugCategories, DebugLine};

/// Collects rapier's segments into the engine's buffer.
struct LineCollector<'a> {
    out: &'a mut Vec<DebugLine>,
}

impl DebugRenderBackend for LineCollector<'_> {
    fn draw_line(&mut self, _object: DebugRenderObject, a: Vec3, b: Vec3, color: DebugColor) {
        self.out.push(DebugLine {
            start: a,
            end: b,
            color: hsla_to_rgb(color),
        });
    }
}

/// Rapier's categories, from ours.
///
/// Not a direct copy of its flag set: `SOLVER_CONTACTS` and `CONTACTS` are
/// separate there — the contacts the solver used this step versus the ones
/// the narrow phase found — and the difference is not one an author is
/// asking about when they tick "contacts". Both go on together, so a
/// contact that exists but was not solved still shows up, which is the
/// interesting case.
fn mode_for(categories: DebugCategories) -> DebugRenderMode {
    let mut mode = DebugRenderMode::empty();
    mode.set(DebugRenderMode::COLLIDER_SHAPES, categories.collider_shapes);
    mode.set(DebugRenderMode::RIGID_BODY_AXES, categories.body_axes);
    mode.set(DebugRenderMode::JOINTS, categories.joints);
    mode.set(DebugRenderMode::COLLIDER_AABBS, categories.collider_aabbs);
    mode.set(
        DebugRenderMode::CONTACTS | DebugRenderMode::SOLVER_CONTACTS,
        categories.contacts,
    );
    mode
}

/// The style, with the one value worth overriding.
///
/// Rapier's defaults are good and its sleep multiplier — which darkens a
/// sleeping body — answers "why did this stop reacting" for free. The
/// tessellation is the exception: 20 subdivisions is ~60 segments per
/// sphere, produced and uploaded every frame, and a debug overlay that
/// costs frame time is one nobody leaves on.
fn style() -> DebugRenderStyle {
    DebugRenderStyle {
        subdivisions: 12,
        ..Default::default()
    }
}

/// HSLA as rapier reports it — hue in degrees, the rest 0..1 — to linear
/// RGB, which is what the line renderer takes.
///
/// The alpha is dropped: the gizmo batch has no blending, and a
/// half-transparent line would silently draw opaque. Better to lose the
/// channel deliberately than to ignore it by accident.
fn hsla_to_rgb([hue, saturation, lightness, _alpha]: DebugColor) -> Vec3 {
    let hue = hue.rem_euclid(360.0);
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);

    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let offset = lightness - chroma / 2.0;
    Vec3::new(r + offset, g + offset, b + offset)
}

impl super::backend::RapierBackend {
    /// Walks the physics world, appending its description to `out`.
    ///
    /// The pipeline is built per call rather than kept on the backend: it
    /// caches shape tessellations, and holding that cache for an overlay
    /// that is off — which is almost always — costs memory for nothing.
    /// When it is on, the caller has already decided to pay for the walk.
    pub(super) fn collect_debug_lines(
        &self,
        categories: DebugCategories,
        out: &mut Vec<DebugLine>,
    ) {
        let mode = mode_for(categories);
        if mode.is_empty() {
            return;
        }
        let mut pipeline = DebugRenderPipeline::new(style(), mode);
        pipeline.render(
            &mut LineCollector { out },
            &self.bodies,
            &self.colliders,
            &self.impulse_joints,
            &self.multibody_joints,
            &self.narrow_phase,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: Vec3, expected: Vec3) -> bool {
        actual.abs_diff_eq(expected, 1e-3)
    }

    /// Rapier's palette is HSLA with hue in degrees, not the 0..1 most
    /// colour code assumes. Reading it as normalised would turn every
    /// collider the same shade.
    #[test]
    fn hue_is_read_in_degrees() {
        assert!(close(
            hsla_to_rgb([0.0, 1.0, 0.5, 1.0]),
            Vec3::new(1.0, 0.0, 0.0)
        ));
        assert!(close(
            hsla_to_rgb([120.0, 1.0, 0.5, 1.0]),
            Vec3::new(0.0, 1.0, 0.0)
        ));
        assert!(close(
            hsla_to_rgb([240.0, 1.0, 0.5, 1.0]),
            Vec3::new(0.0, 0.0, 1.0)
        ));
    }

    #[test]
    fn zero_saturation_is_grey() {
        let grey = hsla_to_rgb([200.0, 0.0, 0.5, 1.0]);
        assert!(close(grey, Vec3::splat(0.5)), "{grey}");
    }

    /// Rapier darkens a sleeping body by scaling its lightness. If that
    /// did not survive the conversion, "why did this stop reacting" stays
    /// unanswerable.
    #[test]
    fn a_darker_lightness_gives_a_darker_colour() {
        let awake = hsla_to_rgb([340.0, 1.0, 0.3, 1.0]);
        let asleep = hsla_to_rgb([340.0, 1.0, 0.3 * 0.2, 1.0]);
        assert!(
            asleep.length() < awake.length(),
            "asleep {asleep} is not darker than awake {awake}",
        );
    }

    /// A hue of exactly 360 is the same colour as 0, and an out-of-range
    /// one must not index past the sector table.
    #[test]
    fn hue_wraps_instead_of_falling_off_the_end() {
        assert!(close(
            hsla_to_rgb([360.0, 1.0, 0.5, 1.0]),
            hsla_to_rgb([0.0, 1.0, 0.5, 1.0]),
        ));
        assert!(hsla_to_rgb([720.0, 1.0, 0.5, 1.0]).is_finite());
        assert!(hsla_to_rgb([-40.0, 1.0, 0.5, 1.0]).is_finite());
    }

    /// Every switch has to reach a rapier flag, or ticking a box in the
    /// editor draws nothing and looks like a broken overlay.
    #[test]
    fn each_category_maps_to_a_rapier_flag() {
        assert!(mode_for(DebugCategories::default()).is_empty());
        let cases = [
            DebugCategories {
                collider_shapes: true,
                ..Default::default()
            },
            DebugCategories {
                contacts: true,
                ..Default::default()
            },
            DebugCategories {
                joints: true,
                ..Default::default()
            },
            DebugCategories {
                collider_aabbs: true,
                ..Default::default()
            },
            DebugCategories {
                body_axes: true,
                ..Default::default()
            },
        ];
        for case in cases {
            assert!(
                !mode_for(case).is_empty(),
                "{case:?} maps to no rapier flag",
            );
        }
        assert_eq!(mode_for(DebugCategories::all()), DebugRenderMode::all());
    }
}
