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
    /// The readers' PCF footprint width, carried to the raster uniform
    /// the shading binds. See `ShadowSettings::page_softness`.
    softness: u32,
    /// The coverage gate (#944). See `ShadowSettings::page_min_pixels`.
    min_pixels: u32,
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

/// How far a count has to move before it is worth another line.
///
/// An eighth. Below that the reader learns nothing the last line did not
/// already say, and the console holds two thousand lines.
const LOG_STEP: u32 = 8;

/// Whether two readings differ by enough to be worth reporting.
fn moved(before: u32, now: u32) -> bool {
    before.abs_diff(now) * LOG_STEP > before.max(now)
}

/// The slot a camera's last logged count lives in, growing the list to
/// reach it.
fn logged<T: Copy>(slots: &mut Vec<Option<T>>, view: u32) -> &mut Option<T> {
    let index = view as usize;
    if slots.len() <= index {
        slots.resize(index + 1, None);
    }
    &mut slots[index]
}

/// This frame's index, from the one clock every camera shares.
///
/// 🔴 Read in two places that run in a fixed order — `bind_page_shadows`
/// before the fused pass and `record_page_marking` after it — so it has
/// to come from the same source in both. `Time::frame_count` is already
/// the stamp the light frame is shared on.
///
/// ⚠️ Without a `Time` — every headless test — it stands still, which
/// pins the uniform's parity. That is the safe direction: both halves
/// then agree on one slice rather than alternating out of step.
fn page_frame(resources: &Resources) -> u32 {
    resources
        .get::<kooch_core::time::Time>()
        .map(|t| t.frame_count() as u32)
        .unwrap_or(0)
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
        // Overwritten by `page_settings_for_views` from the debug view
        // selector. `ShadowSettings` has no say: it is a debug view.
        paint: false,
        density: shadows.page_density,
        softness: shadows.page_softness,
        min_pixels: shadows.page_min_pixels,
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
        debug: crate::meshlet::MeshletDebugMode,
    ) {
        // 🔴 The whole track ran UNPROFILED until now: not one scope
        // between the marking, the four raster passes and the readback,
        // so in the profiler it was time that simply went missing. A
        // pass that cannot be seen cannot be blamed, and the CPU cost of
        // this track was argued about for an hour without one.
        profiling::scope!("shadow pages");
        let settings = self.page_settings_for_views(resources, debug);
        if !settings.enabled {
            self.forget_page_marking();
            // 🔴 Unbind, do not merely stop drawing. The atlas still
            // holds the last frame it filled, and a shading pass that
            // kept sampling it would show a shadow frozen in place —
            // silent, and blamed on everything else first.
            self.lights.unbind_shadow_pages(device);
            return;
        }
        // 🔴 Stamped BEFORE the pool is touched, and once per frame
        // rather than once per camera. `set_pool` sets the rebuild flag
        // and `set_frame` is what clears it, so the other order would
        // clear a rebuild the same frame it was asked for.
        //
        // ⚠️ Without a `Time` — every headless test — the stamp stands
        // still, which means nothing ages and everything stays resident.
        // That is the safe direction to be wrong in: a test sees a pool
        // that never evicts rather than one that evicts constantly.
        if let Some(marker) = self.page_marker.as_mut() {
            marker.set_frame(page_frame(resources));
        }
        // The pool is the memory budget, and changing it changes the
        // atlas. Rebuilt rather than resized: a slot recorded against
        // the old atlas names a different page in the new one, so the
        // rebuild flag evicts every entry before anything reads it.
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
        marker.set_coverage(settings.min_pixels);
        let sun = self.light_frame.as_ref().and_then(|(_, frame)| frame.sun());
        let slice = page_view_index(view_id);
        // 🔴 The CPU scopes above are not the instrument this track
        // needed. Every dispatch below runs on the GPU, and the frame
        // encoder carried exactly two GPU scopes — `cull` and
        // `raster + shade` — with this whole block recorded between
        // them and inside neither. The profiler therefore reported a
        // GPU frame of 11 ms while `drm-engine-gfx` reported 45, and
        // the missing 34 had nowhere to be attributed.
        let scopes = resources.get::<kooch_core::gpu::GpuScopes>();
        let track = scopes.map(|s| s.begin("shadow pages", encoder));
        let view = &self.views[view_id];
        // 🔴 Braced. A `profiling::scope!` lives until the end of its
        // BLOCK, so an unbraced one here would swallow the raster too
        // and the two would be one number again.
        {
            profiling::scope!("mark");
            let query = track
                .as_ref()
                .zip(scopes)
                .map(|(parent, s)| s.begin_child("page mark", encoder, parent));
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
            if let (Some(scopes), Some(query)) = (scopes, query) {
                scopes.end(encoder, query);
            }
        }
        let query = track
            .as_ref()
            .zip(scopes)
            .map(|(parent, s)| s.begin_child("page raster", encoder, parent));
        // The four passes inside nest under this one, so the flamegraph
        // splits the raster into the things that actually scale apart:
        // levels, resident pages, pairs, covered texels.
        let inner = query.as_ref().zip(scopes).map(|(q, s)| (s, q));
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
            inner,
        );
        // 🔴 Both closed unconditionally. `end_frame` rejects a frame
        // that carries an open query and drops EVERY GPU timing with
        // it, so an early return between a `begin` and its `end` blinds
        // the whole profiler, not just this track.
        if let Some(scopes) = scopes {
            if let Some(query) = query {
                scopes.end(encoder, query);
            }
            if let Some(track) = track {
                scopes.end(encoder, track);
            }
        }
    }

    /// The settings, with the live camera count folded in.
    ///
    /// 🔴 `views` is part of the pool's LAYOUT — the atlas is an array
    /// with a layer each — so opening a second viewport rebuilds it, the
    /// same way changing the page budget does.
    fn page_settings_for_views(
        &self,
        resources: &Resources,
        debug: crate::meshlet::MeshletDebugMode,
    ) -> PageSettings {
        let mut settings = page_settings(resources);
        // 🔴 The tile paint is a DEBUG VIEW, so it is driven by the
        // debug view selector and by nothing else. It used to be a
        // separate checkbox in the settings panel, which put the two
        // halves of one question — what marking chose, what the reader
        // found — in two different places for no reason.
        settings.paint = debug == crate::meshlet::MeshletDebugMode::VirtualPageTiles;
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

    /// Records the debug paint, which is the half of the marking that
    /// cannot run at the top of the frame.
    ///
    /// See `PageMarker::record_paint`: it writes the view's FINAL colour
    /// buffer, and at the top of the frame that still holds the last
    /// frame's image.
    pub(super) fn record_page_paint(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view_id: crate::meshlet::render_stage::ViewId,
    ) {
        let Some(marker) = self.page_marker.as_ref() else {
            return;
        };
        marker.record_paint(encoder, self.views[view_id].render_size);
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
        let frame = page_frame(resources);
        let (Some(raster), Some(marker)) = (self.page_raster.as_mut(), self.page_marker.as_ref())
        else {
            return;
        };
        // 🔴 BEFORE the span is asked for, and this is the only place
        // that can do it: the marking that stamps everything else runs
        // after the fused pass, and the parity has to be right now.
        raster.set_frame(frame);
        let pool = marker.pool();
        self.lights.bind_shadow_pages(
            device,
            kooch_lighting::PageBinding {
                uniform: raster.uniform_buffer(),
                // 🔴 THIS frame's slice, now that the raster runs before
                // the shading rather than after it. The parity that used
                // to be needed here is gone with the reason for it: the
                // table, the atlas and the uniform are all this frame's.
                uniform_span: raster.uniform_span(page_view_index(view_id)),
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
        track: crate::shadow::pages::raster::RasterTrack<'_>,
    ) {
        // 🔴 No sun does NOT skip the raster. This gate predated the
        // local raster ("their raster is the next machine") and
        // outlived it: a scene lit only by lamps marked pages, claimed
        // pool slots, and never compacted, drew or aged a thing — the
        // reader then sampled whatever the atlas held last, which in
        // the editor is another scene's pages. Measured as shadows
        // completely broken in every lamp-only scene.
        //
        // The clipmap still wants an orientation for its cull volumes
        // and bucket scale; with no sun the default is straight down,
        // which only has to be CONSISTENT with the marking's own
        // no-sun default — both are `Vec3::NEG_Y`.
        let sun = sun.unwrap_or(Vec3::NEG_Y);
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
        // Already stamped by `bind_page_shadows`, which runs first. Kept
        // here for the frame where the rasteriser was only just built
        // and that call found nothing to stamp.
        raster.set_frame(marker.life().frame);
        raster.set_softness(settings.softness);
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
            self.lights.uploaded(),
            self.lights.light_buffer(),
            &self.moved_casters,
            lod_target,
            track,
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
        self.page_marking_last = Some(counts);
        // 🔴 On a MEANINGFUL change, not on any change. The count moves
        // every frame even with the camera still — the temporal jitter
        // shifts sub-pixel samples into other pages — and the cameras
        // alternate, so an equality check against one shared slot fired
        // twice a frame. The panel has the exact number; this log is for
        // the runs that have no panel.
        let before = logged(&mut self.page_marking_logged, counts.view);
        let notable = before.is_none_or(|last| {
            moved(last.resident, counts.resident)
                || (last.overflow > 0) != (counts.overflow > 0)
                || (last.pool.overflow > 0) != (counts.pool.overflow > 0)
        });
        if !notable {
            return;
        }
        *logged(&mut self.page_marking_logged, counts.view) = Some(counts);
        if counts.overflow > 0 {
            tracing::warn!(
                resident = counts.resident,
                overflow = counts.overflow,
                "shadow pages: the mark buffer is too small, so `resident` is a floor rather than a count"
            );
            return;
        }
        // `debug!`, not `info!`: the throttle above still passes most
        // frames — the counts breathe past an eighth on their own — and
        // at editor rates that is hundreds of console lines a second,
        // which is cost and noise in exactly the runs the panel already
        // serves. The warns above stay loud.
        tracing::debug!(
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
        self.page_raster_last = Some(counts);
        let before = logged(&mut self.page_raster_logged, counts.view);
        let notable = before.is_none_or(|last| {
            moved(last.pages, counts.pages)
                || moved(last.local, counts.local)
                || (last.dropped > 0) != (counts.dropped > 0)
                || (last.overflow > 0) != (counts.overflow > 0)
        });
        // The WARN fires on the TRANSITION into dropping, not on every
        // notable frame: animated lights move the page counts every
        // frame, and each movement re-armed the warn — two thousand
        // identical lines before anyone scrolled.
        let began_failing = before.is_none_or(|last| last.dropped == 0 && last.overflow == 0)
            && (counts.dropped > 0 || counts.overflow > 0);
        if !notable {
            return;
        }
        *logged(&mut self.page_raster_logged, counts.view) = Some(counts);
        if began_failing {
            // Say which failure it is: pages past a bucket's room, or
            // lights past the shadow cap — the fixes are different.
            let cap = crate::shadow::pages::raster::LAMP_CULLS;
            if self.lights.light_count() > cap {
                tracing::warn!(
                    dropped = counts.dropped,
                    lights = self.lights.light_count(),
                    cap,
                    "shadow pages: lights past the cap cast no shadow — their pages are \
                     the dropped count"
                );
                return;
            }
            tracing::warn!(
                dropped = counts.dropped,
                overflow = counts.overflow,
                "shadow pages: the raster ran out of room, so shadows are missing"
            );
            return;
        }
        tracing::debug!(
            view = counts.view,
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
        self.page_marking_logged.clear();
        self.page_raster_logged.clear();
        if let Some(marker) = self.page_marker.as_mut() {
            marker.forget();
        }
    }

    /// What the last dispatch found, for a caller that wants the number
    /// rather than the log.
    pub fn page_marking(&self) -> Option<MarkCounts> {
        self.page_marking_last
    }

    /// The counts belonging to ONE view.
    ///
    /// 🔴 [`Self::page_marking`] is whichever readback landed last, and
    /// with two viewports alive that is a coin toss. The editor drew the
    /// Game tab's overlay out of it and got the Edit view's camera —
    /// same scene, different frustum, and every reading taken from that
    /// panel described a camera nobody was looking through.
    pub fn page_marking_for(&self, view: crate::meshlet::render_stage::ViewId) -> Option<MarkCounts> {
        let want = page_view_index(view);
        self.page_marking_last.filter(|c| c.view == want)
    }

    /// The raster counts belonging to ONE view, for the same reason.
    pub fn page_raster_for(&self, view: crate::meshlet::render_stage::ViewId) -> Option<RasterCounts> {
        let want = page_view_index(view);
        self.page_raster_last.filter(|c| c.view == want)
    }
}

#[cfg(test)]
mod tests;
