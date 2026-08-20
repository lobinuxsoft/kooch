//! Driving the shadow-page marking pass from a frame (#866).
//!
//! 🔴 **An instrument, and off unless asked for.** Nothing reads what
//! the pass writes. It exists to falsify the CPU census in
//! [`crate::shadow::pages`]: that census is a model of how many pages a
//! frame would need, and this is the first thing that can disagree with
//! it. `KOOCH_PAGE_MARKING=1` turns it on, the way `KOOCH_CLUSTERING=off`
//! is the grid's own A/B.

use glam::{Mat4, Vec3};

use kooch_core::resource::Resources;

use crate::shadow::pages::PageMarkingSettings;
use crate::shadow::pages::mark::{MarkCounts, PageMarker, Paint};
use crate::shadow::{ClipmapConfig, PageConfig};

use super::super::stage::MeshletRenderStage;

/// The panel owns this; the environment variable is only its default.
///
/// Absent means nobody inserted the resource, which is every headless
/// test — and off is the right answer there.
fn page_marking_settings(resources: &Resources) -> PageMarkingSettings {
    resources
        .get::<PageMarkingSettings>()
        .copied()
        .unwrap_or(PageMarkingSettings {
            enabled: false,
            rate: 1,
            paint: false,
        })
}

impl MeshletRenderStage {
    /// Records the marking dispatch, building the pass on first use.
    ///
    /// Call **after** the raster wrote depth and after the froxel grid:
    /// the depth says where a surface is, the grid says which lights
    /// reach it, and this reads both.
    pub(super) fn record_page_marking(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        resources: &Resources,
        view_id: crate::meshlet::render_stage::ViewId,
        clip_from_world: Mat4,
        eye: Vec3,
    ) {
        let settings = page_marking_settings(resources);
        if !settings.enabled {
            self.forget_page_marking();
            return;
        }
        let marker = self.page_marker.get_or_insert_with(|| {
            PageMarker::new(device, PageConfig::default(), ClipmapConfig::default())
        });
        let sun = self.light_frame.as_ref().and_then(|(_, frame)| frame.sun());
        let view = &self.views[view_id];
        marker.record(
            device,
            queue,
            encoder,
            &self.lights,
            &view.depth_sample_view,
            clip_from_world.inverse(),
            eye,
            sun,
            view.render_size,
            settings.rate,
            Paint {
                target: &view.color_view,
                on: settings.paint,
                size: view.size,
            },
        );
    }

    /// Maps this frame's counters and logs whatever earlier frames
    /// returned.
    ///
    /// Call **after** the encoder has been submitted: `map_async` before
    /// the submit is a validation error, which is why the readback ring
    /// is split in two halves here and in `ClusterReadback` alike.
    pub(super) fn report_page_marking(&mut self, resources: &Resources) {
        // 🔴 The enablement is checked HERE too, and forgetting it was a
        // bug that made turning the pass OFF log *more*: `record` reset
        // the last-logged count, this kept reading the marker's own
        // cached one, and "did it change?" then answered yes every
        // single frame. A guard on the recording half is not a guard on
        // the reporting half.
        if !page_marking_settings(resources).enabled {
            self.forget_page_marking();
            return;
        }
        let Some(marker) = self.page_marker.as_mut() else {
            return;
        };
        marker.poll();
        let Some(counts) = marker.last() else {
            return;
        };
        // 🔴 On change, not every frame. The count moves with the camera
        // and a per-frame log at sixty hertz is a log nobody reads —
        // the same reason the point-shadow warning is a flag rather than
        // a count.
        if self.page_marking_last == Some(counts) {
            return;
        }
        self.page_marking_last = Some(counts);
        if counts.overflow > 0 {
            tracing::warn!(
                resident = counts.resident,
                overflow = counts.overflow,
                "shadow pages: the mark buffer is too small, so `resident` is a floor rather than a count"
            );
            return;
        }
        tracing::info!(
            resident = counts.resident,
            samples = counts.samples,
            pairs = counts.pairs,
            width = counts.size.0,
            height = counts.size.1,
            "shadow pages marked"
        );
    }

    /// Drops every count the pass produced, so a run that starts again
    /// reports what it finds rather than what it found before.
    fn forget_page_marking(&mut self) {
        self.page_marking_last = None;
        if let Some(marker) = self.page_marker.as_mut() {
            marker.forget();
        }
    }

    /// What the last dispatch found, for a caller that wants the number
    /// rather than the log.
    pub fn page_marking(&self) -> Option<MarkCounts> {
        self.page_marking_last
    }
}
