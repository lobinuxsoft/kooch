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

use crate::meshlet::SceneCullParams;
use crate::shadow::pages::mark::{MarkCounts, PageMarker, Paint};
use crate::shadow::pages::pool::{PAGES_RANGE, PoolConfig};
use crate::shadow::pages::raster::{PageRasterizer, RasterCounts};
use crate::shadow::{ClipmapConfig, PageConfig};

use super::super::stage::MeshletRenderStage;

/// What the project's render settings say about virtual shadow maps.
///
/// 🔴 A public setting now, where #866 kept it a panel-only diagnostic.
/// That restraint was right while nothing read what marking wrote: a
/// knob promising memory nobody spent. The pass is load-bearing now —
/// it decides which shadows exist — so it belongs where the rest of the
/// shadow settings live, in its own group beside the cascades it
/// replaces.
///
/// Absent means nobody inserted the resource, which is every headless
/// test, and off is the right answer there.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct PageSettings {
    enabled: bool,
    paint: bool,
    density: u32,
    pool: PoolConfig,
}

fn page_settings(resources: &Resources) -> PageSettings {
    let Some(render) = resources.get::<crate::settings::RenderSettings>() else {
        return PageSettings {
            enabled: false,
            paint: false,
            density: 100,
            pool: PoolConfig::default(),
        };
    };
    PageSettings {
        // 🔴 `KOOCH_PAGE_MARKING=1` still forces it on, and it survives
        // as a FORCE rather than as a default: the comparison it exists
        // for is made on a handheld, over SSH, against a build nobody
        // wants to make twice.
        enabled: render.virtual_shadows || crate::shadow::pages::mark::enabled_by_environment(),
        paint: render.virtual_shadow_debug,
        density: render.shadow_density,
        pool: PoolConfig {
            pages: render.shadow_pool_pages.clamp(PAGES_RANGE.0, PAGES_RANGE.1),
        },
    }
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
        scene_params: &SceneCullParams,
        meshlet_bg: &wgpu::BindGroup,
    ) {
        let settings = page_settings(resources);
        if !settings.enabled {
            self.forget_page_marking();
            // 🔴 Unbind, do not merely stop drawing. The atlas still
            // holds the last frame it filled, and a shading pass that
            // kept sampling it would show a shadow frozen in place —
            // silent, and blamed on everything else first.
            self.lights.unbind_shadow_pages(device);
            return;
        }
        // The pool is the memory budget, and changing it changes the
        // atlas. Rebuilt rather than resized: it is emptied every frame
        // anyway, so there is nothing in it worth carrying across.
        if self.page_pool_config != Some(settings.pool) {
            self.page_pool_config = Some(settings.pool);
            self.page_raster = None;
            self.lights.unbind_shadow_pages(device);
            if let Some(marker) = self.page_marker.as_mut() {
                marker.set_pool(device, settings.pool);
            }
        }
        let marker = self.page_marker.get_or_insert_with(|| {
            let mut marker =
                PageMarker::new(device, PageConfig::default(), ClipmapConfig::default());
            marker.set_pool(device, settings.pool);
            marker
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
            // 🔴 Always one sample per pixel. While this was an
            // instrument a coarser rate traded accuracy for threads;
            // now it decides which pages EXIST, and one sample in
            // sixteen is fifteen pixels whose shadow was never
            // rasterised.
            1,
            settings.density,
            Paint {
                target: &view.color_view,
                on: settings.paint,
                size: view.size,
            },
        );
        self.record_page_raster(
            device,
            queue,
            encoder,
            settings,
            sun,
            eye,
            scene_params,
            meshlet_bg,
        );
    }

    /// Rasterises depth into the pages the dispatch above just marked.
    ///
    /// 🔴 Ordered right after marking and not with the cascades: it
    /// reads the page table THIS frame's depth buffer filled, so it
    /// cannot run before the depth pass the way a cascade can.
    #[allow(clippy::too_many_arguments)]
    fn record_page_raster(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        settings: PageSettings,
        sun: Option<Vec3>,
        eye: Vec3,
        scene_params: &SceneCullParams,
        meshlet_bg: &wgpu::BindGroup,
    ) {
        // No sun is no clipmap. Local lights are marked and allocated
        // and their raster is the next machine — see `pages::raster`.
        let Some(sun) = sun else {
            return;
        };
        let (Some(pool), Some(marker)) = (self.gpu_pool.as_ref(), self.page_marker.as_ref()) else {
            return;
        };
        // 🔴 One texel of simplification error, and NOT the camera's
        // LOD target. A clipmap level is already a texel density, and
        // the cull is handed that density directly — applying the
        // screen's target on top would be the relaxation this project
        // already removed from the cascades for being applied twice.
        let lod_target = 1.0_f32;
        let page_pool = marker.pool();
        let lights = self.lights.light_count().max(1);
        let raster = self.page_raster.get_or_insert_with(|| {
            PageRasterizer::new(
                device,
                self.cull_pipelines.meshlet_bind_group_layout(),
                PageConfig::default(),
                ClipmapConfig::default(),
                settings.pool,
                super::super::super::DEFAULT_MAX_TRIANGLES as u32,
            )
        });
        // Groups are bounded by meshlets, and a slot is four bytes: the
        // bound costs less than threading the exact figure through a
        // second call path would.
        let threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
        raster.ensure_capacity(device, threads, threads);
        raster.record(
            device,
            queue,
            encoder,
            &self.cull_pipelines,
            pool,
            &self.scene,
            meshlet_bg,
            self.scene.instance_buffer(),
            page_pool,
            scene_params,
            eye,
            sun,
            lights,
            lod_target,
        );
        // Idempotent, and this is the one call site that runs after
        // every possible rebuild of either side.
        self.lights.bind_shadow_pages(
            device,
            kooch_lighting::PageBinding {
                uniform: raster.uniform_buffer(),
                keys: page_pool.keys(),
                slots: page_pool.slots(),
                atlas: raster.atlas(),
            },
        );
    }

    /// What the raster did, for the panel.
    pub fn page_raster(&self) -> Option<RasterCounts> {
        self.page_raster_last
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
        if !page_settings(resources).enabled {
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

    /// The same, for the raster's own counters.
    pub(super) fn report_page_raster(&mut self, resources: &Resources) {
        if !page_settings(resources).enabled {
            self.page_raster_last = None;
            return;
        }
        let Some(raster) = self.page_raster.as_mut() else {
            return;
        };
        let Some(counts) = raster.poll() else {
            return;
        };
        if self.page_raster_last == Some(counts) {
            return;
        }
        self.page_raster_last = Some(counts);
        if counts.dropped > 0 || counts.overflow > 0 {
            tracing::warn!(
                dropped = counts.dropped,
                overflow = counts.overflow,
                "shadow pages: the raster ran out of room, so shadows are missing"
            );
            return;
        }
        tracing::info!(
            pages = counts.pages,
            pairs = counts.pairs,
            local = counts.local,
            "shadow pages rastered"
        );
    }

    /// Drops every count the pass produced, so a run that starts again
    /// reports what it finds rather than what it found before.
    fn forget_page_marking(&mut self) {
        self.page_marking_last = None;
        self.page_raster_last = None;
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
