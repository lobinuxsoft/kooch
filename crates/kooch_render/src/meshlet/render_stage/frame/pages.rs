//! Driving the shadow-page marking pass from a frame (#866).

use glam::{Mat4, Vec3};

use kooch_core::resource::Resources;

use crate::meshlet::SceneCullParams;
use crate::shadow::pages::Casters;
use crate::shadow::pages::mark::{MarkCounts, PageMarker, Paint};
use crate::shadow::pages::pool::{PAGES_RANGE, PoolConfig};
use crate::shadow::pages::raster::{PageRasterizer, RasterCounts};
use crate::shadow::{ClipmapConfig, PageConfig};

use super::super::stage::MeshletRenderStage;

/// What the project's render settings say about virtual shadow maps.
#[derive(Copy, Clone, Debug, PartialEq)]
struct PageSettings {
    enabled: bool,
    paint: bool,
    density: u32,
    pool: PoolConfig,
    /// The readers' PCF footprint width, carried to the raster uniform
    /// the shading binds. See `ShadowSettings::page_softness`.
    softness: u32,
    /// The readers' bias: normal step per texel, depth step in metres, the metre ceiling on the
    /// first, and the ceiling on the receiver's own depth GRADIENT (#1017) — the term that gives
    /// each filter tap the depth its own part of the receiving plane has.
    bias: (f32, f32, f32, f32),
    /// The coverage gate (#944). See `ShadowSettings::page_min_pixels`.
    /// Whether the shading marches the atlas (#1017).
    march: bool,
    /// Whether the expansion runs from the geometry (#1022).
    geometry: bool,
    /// How far, in pages, a receiver dilates its request (#1022).
    halo: f32,
    min_pixels: u32,
    /// The distance gate. See `ShadowSettings::page_light_reach`.
    reach: u32,
    /// How many times the set of loaded scenes has changed.
    scene_epoch: u32,
    /// Whether the clipmap culls enter per instance (#1002).
    two_level: bool,
}

/// A camera's index into the pool's slices.
pub(super) fn page_view_index(id: crate::meshlet::render_stage::ViewId) -> u32 {
    use slotmap::Key;
    ((id.data().as_ffi() & 0xffff_ffff) as u32).saturating_sub(1)
}

/// How far a count has to move before it is worth another line.
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
fn page_frame(resources: &Resources) -> u32 {
    resources
        .get::<kooch_core::time::Time>()
        .map(|t| t.frame_count() as u32)
        .unwrap_or(0)
}

/// How many frames a page may go unrequested, for THIS frame rate.
fn page_age_frames(resources: &Resources) -> u32 {
    let Some(delta) = resources
        .get::<kooch_core::time::Time>()
        .map(|t| t.delta_secs())
        .filter(|d| *d > 0.0)
    else {
        // No clock: keep the documented default rather than invent one.
        return crate::shadow::pages::pool::age_from_environment();
    };
    age_frames(crate::shadow::pages::pool::age_seconds(), delta)
}

/// The conversion itself, split out so it is testable without a clock.
fn age_frames(seconds: f32, delta: f32) -> u32 {
    ((seconds / delta).ceil() as u32).clamp(AGE_FRAMES_MIN, AGE_FRAMES_MAX)
}

/// Floor and ceiling on the converted horizon. See [`page_age_frames`].
const AGE_FRAMES_MIN: u32 = 30;
const AGE_FRAMES_MAX: u32 = 1024;

/// The scene epoch, as the page machine can see it from here.
fn read_epoch(resources: &Resources) -> u32 {
    let manager = resources.get::<kooch_ecs::SceneManager>();
    let epoch = manager.map(|m| m.epoch()).unwrap_or(0);
    let at = manager.map_or(0, |m| m as *const _ as usize);
    // Packed so one atomic carries both halves: a reader that found
    // nothing and a reader that found zero must not collapse.
    let seen = (u64::from(manager.is_some()) << 32) | u64::from(epoch);
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    if LAST.swap(seen, std::sync::atomic::Ordering::Relaxed) != seen {
        tracing::info!(
            target: "kooch_render::shadow",
            found = manager.is_some(),
            epoch,
            manager = at,
            "the page machine read the scene epoch",
        );
    }
    epoch
}

fn page_settings(resources: &Resources) -> PageSettings {
    // 🔴 `ShadowSettings`, not `RenderSettings`, and `unwrap_or_default` rather than an early
    // return. Both halves of that were the bug.
    let shadows = resources
        .get::<crate::shadow::ShadowSettings>()
        .copied()
        .unwrap_or_default();
    let lod = resources
        .get::<crate::meshlet::MeshletLodSettings>()
        .copied()
        .unwrap_or_default();
    PageSettings {
        two_level: lod.two_level,
        // 🔴 The environment force is ORed HERE **as well as** in `RenderSettings::shadows()`, and
        // the duplication is the point.
        enabled: shadows.virtual_pages || crate::shadow::pages::mark::enabled_by_environment(),
        // Overwritten by `page_settings_for_views` from the debug view
        // selector. `ShadowSettings` has no say: it is a debug view.
        paint: false,
        density: shadows.page_density,
        softness: shadows.page_softness,
        bias: (
            shadows.page_normal_bias,
            shadows.page_depth_bias,
            shadows.page_bias_max,
            shadows.page_bias_slope,
        ),
        march: shadows.page_march,
        geometry: shadows.page_geometry,
        halo: shadows.page_halo,
        min_pixels: shadows.page_min_pixels,
        reach: shadows.page_light_reach,
        // Absent in a headless test and in any host without a manager,
        // where zero is right: nothing ever changes, so nothing ever
        // needs voiding.
        scene_epoch: read_epoch(resources),
        pool: PoolConfig {
            pages: shadows.pool_pages.clamp(PAGES_RANGE.0, PAGES_RANGE.1),
            // Filled in by the caller, which is the only place that
            // knows how many cameras are alive.
            views: 1,
            // Likewise: the device's texture limit arrives with the
            // device, in `fit_atlas`.
            row_cap: u32::MAX,
        },
    }
}

impl MeshletRenderStage {
    /// The settings, with the live camera count folded in.
    fn page_settings_for_views(
        &self,
        resources: &Resources,
        debug: crate::meshlet::MeshletDebugMode,
    ) -> PageSettings {
        let mut settings = page_settings(resources);
        // 🔴 The tile paint is a DEBUG VIEW, so it is driven by the debug view selector and by
        // nothing else.
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

    /// Records the debug paint, which is the half of the marking that cannot run at the top of the
    /// frame.
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
                // 🔴 THIS frame's slice, now that the raster runs before the shading rather than
                // after it. The parity that used to be needed here is gone with the reason for it:
                // the table, the atlas and the uniform are all this frame's.
                uniform_span: raster.uniform_span(page_view_index(view_id)),
                slots: pool.slots(),
                atlas: raster.atlas(),
            },
        );
    }

    /// What the raster did, for the panel.
    pub fn page_raster(&self) -> Option<RasterCounts> {
        self.page_raster_last
    }

    /// Maps this frame's counters and logs whatever earlier frames returned.
    pub(super) fn report_page_marking(&mut self, resources: &Resources) {
        // 🔴 The enablement is checked HERE too, and forgetting it was a bug that made turning the
        // pass OFF log *more*: `record` reset the last-logged count, this kept reading the marker's
        // own cached one, and "did it change?" then answered yes every single frame.
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
        // 🔴 On a MEANINGFUL change, not on any change. The count moves every frame even with the
        // camera still — the temporal jitter shifts sub-pixel samples into other pages — and the
        // cameras alternate, so an equality check against one shared slot fired twice a frame.
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
        // `debug!`, not `info!`: the throttle above still passes most frames — the counts breathe
        // past an eighth on their own — and at editor rates that is hundreds of console lines a
        // second, which is cost and noise in exactly the runs the panel already serves.
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
        // The WARN fires on the TRANSITION into dropping, not on every notable frame: animated
        // lights move the page counts every frame, and each movement re-armed the warn — two
        // thousand identical lines before anyone scrolled.
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

    /// Gives the whole page machine back: the atlas, the flat table, the per-view free lists, every
    /// pipeline's buffers.
    fn release_pages(&mut self, device: &wgpu::Device) {
        if self.page_marker.is_none() && self.page_raster.is_none() {
            return;
        }
        self.forget_page_marking();
        self.lights.unbind_shadow_pages(device);
        self.page_marker = None;
        self.page_raster = None;
        // The pool the atlas WAS built for, and there is no atlas now.
        // Left set, the next build would skip `set_pool` and run against
        // a marker that never sized its table.
        self.page_pool_config = None;
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
    pub fn page_marking_for(
        &self,
        view: crate::meshlet::render_stage::ViewId,
    ) -> Option<MarkCounts> {
        let want = page_view_index(view);
        self.page_marking_last.filter(|c| c.view == want)
    }

    /// The raster counts belonging to ONE view, for the same reason.
    pub fn page_raster_for(
        &self,
        view: crate::meshlet::render_stage::ViewId,
    ) -> Option<RasterCounts> {
        let want = page_view_index(view);
        self.page_raster_last.filter(|c| c.view == want)
    }
}

mod record;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod age_horizon_tests;
impl PageSettings {
    /// The same settings with a pool one atlas layer can actually hold.
    fn fit_atlas(mut self, max_side: u32) -> Self {
        self.pool = self.pool.fit_atlas(max_side, PageConfig::default().page);
        self
    }
}
