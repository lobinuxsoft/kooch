//! Where the editor's CPU frame actually goes (#691).

use std::time::Instant;

use kooch_core::resource::Resources;

use super::EditorPerfStats;

/// Milliseconds elapsed since `start`, in the shape the HUD wants.
pub(crate) fn ms_since(start: Instant) -> f32 {
    start.elapsed().as_secs_f32() * 1000.0
}

/// What the gather stage spends its time on.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GatherStages {
    /// Resolving every registered component name to a stable id, before
    /// the read-only gathers below can use one.
    pub intern_ms: f32,
    /// Every entity with its components and their reflected field
    /// values. Grows with the world twice over: entities × components.
    pub entities_ms: f32,
    /// The archetype list for the Components panel.
    pub archetypes_ms: f32,
    /// The registered-type lists behind "Add Component".
    pub types_ms: f32,
    /// The asset catalog for the Inspector's pickers, and the contents
    /// of whatever the Asset Browser has selected.
    pub assets_ms: f32,
}

impl GatherStages {
    /// What the sub-stages add up to. The difference from `gather_ms` is
    /// the scene snapshot and the resource shuffling around them.
    pub fn total_ms(&self) -> f32 {
        self.intern_ms + self.entities_ms + self.archetypes_ms + self.types_ms + self.assets_ms
    }
}

/// The render system's own stages, in the order the frame runs them.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RenderStages {
    /// Building the frame's read-only view of the world for the UI:
    /// hierarchy, inspector data, asset catalog, selected-asset detail.
    /// Walks every entity, so it grows with the scene.
    pub gather_ms: f32,
    /// What that time went on.
    pub gather: GatherStages,
    /// The egui pass. Every panel's contents, laid out and painted —
    /// immediate mode, so a list of 610 rows costs 610 rows every frame
    /// whether or not one of them changed.
    pub ui_ms: f32,
    /// Viewport input: gizmo handles, picking, camera. Cheap unless the
    /// pointer is doing something, which is exactly when it matters.
    pub input_ms: f32,
    /// Recording the viewport's GPU work — sky, meshlet stage, gizmo
    /// batches, blit. CPU-side command encoding only; the GPU's own time
    /// is `gpu_frame_ms` and is not in here.
    pub viewport_ms: f32,
    /// Handing the frame to the surface, including egui's tessellation
    /// and texture uploads.
    pub present_ms: f32,
    /// Applying the actions the UI queued: spawns, despawns, component
    /// edits, saves. Zero on a frame where the user did nothing.
    pub actions_ms: f32,
}

/// Per-stage cost of one editor frame.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FrameBreakdown {
    /// The render system's stages.
    pub render: RenderStages,
    /// Building the gizmo line + mesh batches, in `Stage::PreRender`.
    /// Outside `cpu_frame_ms`; see the module docs.
    pub gizmo_batch_ms: f32,
}

impl RenderStages {
    /// What the six stages add up to.
    pub fn total_ms(&self) -> f32 {
        self.gather_ms
            + self.ui_ms
            + self.input_ms
            + self.viewport_ms
            + self.present_ms
            + self.actions_ms
    }
}

impl FrameBreakdown {
    /// The part of `cpu_frame_ms` no stage claims.
    pub fn residual_ms(&self, cpu_frame_ms: f32) -> f32 {
        (cpu_frame_ms - self.render.total_ms()).max(0.0)
    }
}

/// Publishes the render system's stages. Called once, at the end of the
/// frame, next to [`super::record_cpu_frame_ms`].
pub fn record_render_stages(resources: &mut Resources, stages: RenderStages) {
    if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
        stats.breakdown.render = stages;
    }
}

/// Publishes the gizmo batch's cost from its own system.
pub fn record_gizmo_batch_ms(resources: &mut Resources, ms: f32) {
    if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
        stats.breakdown.gizmo_batch_ms = ms;
    }
}

#[cfg(test)]
mod tests;
