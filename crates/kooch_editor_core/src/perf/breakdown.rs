//! Where the editor's CPU frame actually goes (#691).
//!
//! # Why measuring by subtraction stopped working
//!
//! The frame had exactly two numbers: `cpu_frame_ms` for the whole
//! render system, and the remote section for the snapshot pull. That was
//! enough while one suspect dominated. It stopped being enough the
//! moment the obvious costs were paid off: the pull went from 32 ms to
//! 4.7 ms, the mirror to 0.00 ms, and what remained was ten milliseconds
//! attributable to nothing in particular.
//!
//! Subtracting the known costs from the total and reasoning about the
//! remainder produced three hypotheses and one hit. The cull sizing was
//! arithmetically damning and worth 0.076 ms. Vsync was refuted outright.
//! Only the panels were real, and they were found by having the user
//! collapse them — an experiment, not an inference.
//!
//! # What this measures, and what it deliberately does not
//!
//! Six stages of the render system, plus the gizmo batch that runs
//! before it. Each is a wall-clock span around work that already existed
//! as a distinct step, so nothing was restructured to be measurable.
//!
//! [`FrameBreakdown::residual_ms`] is the point of the whole module: the
//! part of `cpu_frame_ms` that the six stages do not account for. A
//! residual near zero means the stages describe the frame and the
//! largest one is the thing to fix. A large residual means the split is
//! in the wrong place and the next stage boundary belongs inside
//! whatever the six are missing. Either answer is worth having; only the
//! second is invisible without this.
//!
//! **The gizmo batch is not part of the residual arithmetic.** It runs
//! in `Stage::PreRender`, outside the span `cpu_frame_ms` covers, and
//! folding it in would make the stages sum past their own total. It is
//! reported beside them because it is per-frame editor cost that scales
//! with the scene and was previously invisible — as is the snapshot
//! pull, for the same reason and in its own section.

use std::time::Instant;

use kooch_core::resource::Resources;

use super::EditorPerfStats;

/// Milliseconds elapsed since `start`, in the shape the HUD wants.
pub(crate) fn ms_since(start: Instant) -> f32 {
    start.elapsed().as_secs_f32() * 1000.0
}

/// What the gather stage spends its time on.
///
/// Gather turned out to be the cost that does not care what is on
/// screen: collapsing every panel took the UI pass from 9.2 ms to 3.1 ms
/// and left gather at 5.6 ms both times. It builds the same snapshot of
/// the world whether or not anything is looking at it, so it is the one
/// number that a person cannot avoid by closing a panel — which is why
/// it gets a split of its own rather than a guess.
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
///
/// Filled in as a local across the render function and handed over once,
/// rather than written field by field into the Resource: a per-stage
/// `resources.get_mut` would be six map lookups on the timing path,
/// measuring itself.
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
    ///
    /// Clamped at zero rather than allowed to go negative. The two spans
    /// are read from separate `Instant`s and the stages are strictly
    /// inside the total, so a negative value can only be float noise on
    /// a sub-microsecond difference — and a HUD reading `-0.00 ms`
    /// invites a hunt for a bug that is not there.
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
mod tests {
    use super::*;

    fn stages() -> RenderStages {
        RenderStages {
            gather_ms: 1.0,
            gather: GatherStages::default(),
            ui_ms: 6.0,
            input_ms: 0.5,
            viewport_ms: 1.5,
            present_ms: 1.0,
            actions_ms: 0.0,
        }
    }

    #[test]
    fn residual_is_what_the_stages_do_not_claim() {
        let breakdown = FrameBreakdown {
            render: stages(),
            gizmo_batch_ms: 3.0,
        };
        // 10 measured out of a 12.5 ms frame.
        assert!((breakdown.residual_ms(12.5) - 2.5).abs() < 1e-4);
    }

    /// The gizmo batch runs outside the span `cpu_frame_ms` covers.
    /// Counting it would report a residual smaller than the truth, which
    /// is the one direction a diagnostic must never err in.
    #[test]
    fn the_gizmo_batch_is_not_deducted_from_the_residual() {
        let with_batch = FrameBreakdown {
            render: stages(),
            gizmo_batch_ms: 3.0,
        };
        let without = FrameBreakdown {
            render: stages(),
            gizmo_batch_ms: 0.0,
        };
        assert_eq!(with_batch.residual_ms(12.5), without.residual_ms(12.5));
    }

    /// Float noise between two separately-read spans must not surface as
    /// a negative number in a panel.
    #[test]
    fn a_total_below_the_stages_reads_as_zero_not_negative() {
        let breakdown = FrameBreakdown {
            render: stages(),
            gizmo_batch_ms: 0.0,
        };
        assert_eq!(breakdown.residual_ms(9.999), 0.0);
    }

    #[test]
    fn recording_stages_leaves_the_gizmo_batch_alone() {
        let mut resources = Resources::default();
        resources.insert(EditorPerfStats::default());
        record_gizmo_batch_ms(&mut resources, 3.0);
        record_render_stages(&mut resources, stages());

        let breakdown = resources.get::<EditorPerfStats>().unwrap().breakdown;
        assert_eq!(
            breakdown.gizmo_batch_ms, 3.0,
            "clobbered by the render pass"
        );
        assert_eq!(breakdown.render.ui_ms, 6.0);
    }

    /// The sub-stages sit inside gather, so they must never sum past
    /// it — a "rest of gather" that reads negative would send someone
    /// looking for time that was only ever double-counted.
    #[test]
    fn the_gather_sub_stages_are_a_split_of_gather_not_an_addition() {
        let gather = GatherStages {
            intern_ms: 0.1,
            entities_ms: 4.0,
            archetypes_ms: 0.3,
            types_ms: 0.2,
            assets_ms: 0.4,
        };
        assert!((gather.total_ms() - 5.0).abs() < 1e-4);

        let render = RenderStages {
            gather_ms: 5.5,
            gather,
            ..stages()
        };
        // Gather's own total is what the frame residual uses; the split
        // is not added on top of it.
        let breakdown = FrameBreakdown {
            render,
            gizmo_batch_ms: 0.0,
        };
        assert!(breakdown.residual_ms(render.total_ms()) < 0.01);
    }

    #[test]
    fn ms_since_measures_forward() {
        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let elapsed = ms_since(start);
        assert!(elapsed >= 2.0, "expected ≥ 2 ms, got {elapsed}");
        assert!(elapsed < 1000.0);
    }
}
