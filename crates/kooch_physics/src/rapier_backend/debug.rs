//! Adapter from rapier's `DebugRenderPipeline` — pre-tessellated [`DebugRenderBackend::draw_line`]
//! calls — to engine segments with a colour conversion. Behind `debug-render`, so shipped games
//! never contain it (#558).

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

/// Rapier's categories from ours; solver and narrow-phase contacts go on together, so an unsolved
/// contact still shows.
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

/// Rapier's style, keeping the sleep darkening, but fewer subdivisions: 20 is ~60 segments per
/// sphere every frame.
fn style() -> DebugRenderStyle {
    DebugRenderStyle {
        subdivisions: 12,
        ..Default::default()
    }
}

/// Rapier's HSLA to linear RGB. Alpha dropped deliberately: the gizmo batch has no blending.
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
    /// Walks the world into `out`, building the pipeline per call so an off overlay holds no
    /// tessellation cache.
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
