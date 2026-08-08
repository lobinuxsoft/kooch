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
mod tests;
