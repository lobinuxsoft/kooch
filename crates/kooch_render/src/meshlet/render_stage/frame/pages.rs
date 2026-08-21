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

/// A camera's index into the pool's slices.
///
/// The slot map's own index, minus the sentinel it reserves at zero.
/// Dense while views live and stable across frames — the two properties
/// a slice needs. Deliberately NOT the position in an iteration order,
/// which would move a camera onto the other one's pages the moment a
/// view was destroyed.
pub(super) fn page_view_index(id: crate::meshlet::render_stage::ViewId) -> u32 {
    use slotmap::Key;
    ((id.data().as_ffi() & 0xffff_ffff) as u32).saturating_sub(1)
}

fn page_settings(resources: &Resources) -> PageSettings {
    // 🔴 `ShadowSettings`, not `RenderSettings`, and `unwrap_or_default`
    // rather than an early return. Both halves of that were the bug.
    //
    // `RenderSettings` is NEVER inserted as a `Resources` value —
    // `apply` publishes derived structs like this one instead — so the
    // lookup returned `None` in every build and the early return took a
    // hardcoded `enabled: false` with it. The environment force sat
    // behind that return and never ran either. The feature shipped
    // inert, and the profile that found it showed a capture with the
    // pages on and one with them off that were identical scope for
    // scope.
    //
    // Absence means defaults, the way `shadows: prepare` has always
    // read this same resource. A missing settings asset is the normal
    // case, not a reason to turn a feature off.
    let shadows = resources
        .get::<crate::shadow::ShadowSettings>()
        .copied()
        .unwrap_or_default();
    PageSettings {
        // 🔴 The environment force is ORed HERE **as well as** in
        // `RenderSettings::shadows()`, and the duplication is the point.
        // `shadows()` only runs when the project HAS a settings asset —
        // `apply_render_settings_system` returns early when it does not
        // — so a force that lived only there would be silently absent
        // from exactly the project it exists for: a scene with no
        // settings file, on a handheld, over SSH. A force is a force
        // wherever the settings came from.
        enabled: shadows.virtual_pages || crate::shadow::pages::mark::enabled_by_environment(),
        paint: shadows.page_debug,
        density: shadows.page_density,
        pool: PoolConfig {
            pages: shadows.pool_pages.clamp(PAGES_RANGE.0, PAGES_RANGE.1),
            // Filled in by the caller, which is the only place that
            // knows how many cameras are alive.
            views: 1,
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
        let settings = self.page_settings_for_views(resources);
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
        let slice = page_view_index(view_id);
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
            slice,
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
            slice,
            sun,
            eye,
            scene_params,
            meshlet_bg,
        );
    }

    /// The settings, with the live camera count folded in.
    ///
    /// 🔴 `views` is part of the pool's LAYOUT — the atlas is an array
    /// with a layer each — so opening a second viewport rebuilds it, the
    /// same way changing the page budget does.
    fn page_settings_for_views(&self, resources: &Resources) -> PageSettings {
        let mut settings = page_settings(resources);
        let slices = self
            .views
            .keys()
            .map(page_view_index)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        settings.pool = settings.pool.with_views(slices);
        settings
    }

    /// Points the shading model at THIS camera's pages.
    ///
    /// 🔴 Called before the fused raster, not after it. `vbuf64.render`
    /// rasterises and shades in one fragment shader, so the bind group
    /// it reads is whatever was left there — and what was left there was
    /// the OTHER camera's slice of the uniform, whose clipmap is centred
    /// on the other camera. One viewport with shadows and one without
    /// is what that looks like.
    ///
    /// The table and atlas it points at are a frame old, which is the
    /// standing limitation of a fused raster and not this call's doing.
    pub(super) fn bind_page_shadows(
        &mut self,
        device: &wgpu::Device,
        resources: &Resources,
        view_id: crate::meshlet::render_stage::ViewId,
    ) {
        if !page_settings(resources).enabled {
            return;
        }
        let (Some(raster), Some(marker)) = (self.page_raster.as_ref(), self.page_marker.as_ref())
        else {
            return;
        };
        let pool = marker.pool();
        self.lights.bind_shadow_pages(
            device,
            kooch_lighting::PageBinding {
                uniform: raster.uniform_buffer(),
                uniform_span: raster.uniform_span(page_view_index(view_id)),
                keys: pool.keys(),
                slots: pool.slots(),
                atlas: raster.atlas(),
            },
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
        slice: u32,
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
            slice,
            eye,
            sun,
            lights,
            lod_target,
        );
        // Idempotent, and this is the one call site that runs after
        // every possible rebuild of either side. The binding the
        // SHADING reads is set by `bind_page_shadows` before the fused
        // pass; this one only makes sure a rebuilt atlas or table is
        // picked up at all.
        self.lights.bind_shadow_pages(
            device,
            kooch_lighting::PageBinding {
                uniform: raster.uniform_buffer(),
                uniform_span: raster.uniform_span(slice),
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
            view = counts.view,
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
            view = counts.view,
            pages = counts.pages,
            pairs = counts.pairs,
            local = counts.local,
            others = counts.others,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The first camera owns the first slice.
    ///
    /// 🔴 A slot map reserves index zero for its null key, so the first
    /// real view is index 1 — and a slice numbering that forgot it would
    /// leave slice 0 permanently unused and put the last camera one past
    /// the end of the pool.
    #[test]
    fn the_first_view_owns_the_first_slice() {
        let mut views: slotmap::SlotMap<crate::meshlet::render_stage::ViewId, u32> =
            slotmap::SlotMap::with_key();
        let first = views.insert(0);
        let second = views.insert(1);
        assert_eq!(page_view_index(first), 0);
        assert_eq!(page_view_index(second), 1);
        // Destroying and recreating hands the slot back, so a camera's
        // slice is stable rather than a position in an iteration order.
        views.remove(first);
        let third = views.insert(2);
        assert_eq!(page_view_index(third), 0);
    }

    #[test]
    fn no_settings_asset_means_defaults_not_disabled() {
        // 🔴 The half of the bug that is testable without touching the
        // environment. The original read took an early return with a
        // hardcoded `enabled: false` whenever the resource was absent —
        // which was every build. A project with no settings asset is
        // the normal case, so absence has to mean DEFAULTS.
        let resources = Resources::default();
        let settings = page_settings(&resources);
        let defaults = crate::shadow::ShadowSettings::default();
        assert_eq!(settings.density, defaults.page_density);
        assert_eq!(settings.pool.pages, defaults.pool_pages);
        assert_eq!(settings.paint, defaults.page_debug);
    }
}
