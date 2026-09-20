//! Recording the shadow pages' marking and raster into a camera's frame.

use super::*;

impl MeshletRenderStage {
    /// Records the marking dispatch, building the pass on first use.
    pub(in crate::meshlet::render_stage::frame) fn record_page_marking(
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
        // 🔴 The whole track ran UNPROFILED until now: not one scope between the marking, the four
        // raster passes and the readback, so in the profiler it was time that simply went missing.
        profiling::scope!("shadow pages");
        // 🔴 Clamped HERE and nowhere later: `per_row` is the page ADDRESSING, so the atlas, the
        // table and every shader that resolves a page id have to agree on one number. Fitting the
        // texture alone would leave the addressing describing a layer that does not exist.
        let settings = self
            .page_settings_for_views(resources, debug)
            .fit_atlas(device.limits().max_texture_dimension_2d);
        if !settings.enabled {
            self.release_pages(device);
            return;
        }
        // 🔴 Read from the light frame rather than counted here, so a light switched off in the
        // inspector and a light despawned with its scene are the same event: `LightFrame::extract`
        // drops both, and this is downstream of it.
        let casters = self
            .light_frame
            .as_ref()
            .map(|(_, frame)| Casters::of_frame(frame));
        if casters.is_some_and(|c| c.is_empty()) {
            // Nothing casts, so no page will ever be requested: give the
            // atlas, the table and the free lists back and record
            // nothing at all until a light returns.
            self.page_casters = casters;
            self.release_pages(device);
            return;
        }
        // 🔴 Stamped BEFORE the pool is touched, and once per frame rather than once per camera.
        // `set_pool` sets the rebuild flag and `set_frame` is what clears it, so the other order
        // would clear a rebuild the same frame it was asked for.
        if let Some(marker) = self.page_marker.as_mut() {
            marker.set_frame(page_frame(resources));
            marker.set_max_age(page_age_frames(resources));
        }
        // 🔴 AFTER `set_frame` and never before it: a new frame index clears the rebuild flag, so
        // voiding first would void nothing. The same ordering trap `set_pool` is commented for, one
        // lever over.
        let scene_changed = self
            .page_epoch
            .replace(settings.scene_epoch)
            .is_some_and(|before| before != settings.scene_epoch);
        let caster_lost = casters
            .zip(self.page_casters)
            .is_some_and(|(now, before)| now.lost(before));
        if let Some(casters) = casters {
            self.page_casters = Some(casters);
        }
        if (scene_changed || caster_lost)
            && let Some(marker) = self.page_marker.as_mut()
        {
            // 🔴 Said out loud, because everything this lever does happens on the GPU and leaves no
            // number behind.
            tracing::info!(
                target: "kooch_render::shadow",
                epoch = settings.scene_epoch,
                scene_changed,
                caster_lost,
                "voiding the shadow page table",
            );
            marker.void();
        }
        // The pool is the memory budget, and changing it changes the atlas. Rebuilt rather than
        // resized: a slot recorded against the old atlas names a different page in the new one, so
        // the rebuild flag evicts every entry before anything reads it.
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
        marker.set_halo(settings.halo);
        marker.set_reach(settings.reach);
        let sun = self.light_frame.as_ref().and_then(|(_, frame)| frame.sun());
        let sun_shadow_layers = self
            .light_frame
            .as_ref()
            .map_or(u32::MAX, |(_, frame)| frame.sun_shadow_layers());
        let slice = page_view_index(view_id);
        // 🔴 The CPU scopes above are not the instrument this track needed. Every dispatch below
        // runs on the GPU, and the frame encoder carried exactly two GPU scopes — `cull` and
        // `raster + shade` — with this whole block recorded between them and inside neither.
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
                // 🔴 Always one sample per pixel. While this was an instrument a coarser rate traded
                // accuracy for threads; now it decides which pages EXIST, and one sample in sixteen
                // is fifteen pixels whose shadow was never rasterised.
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
            sun_shadow_layers,
            eye,
            scene_params,
            meshlet_bg,
            inner,
        );
        // 🔴 Both closed unconditionally. `end_frame` rejects a frame that carries an open query and
        // drops EVERY GPU timing with it, so an early return between a `begin` and its `end` blinds
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

    /// Rasterises depth into the pages the dispatch above just marked.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_page_raster(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        settings: PageSettings,
        slice: u32,
        sun: Option<Vec3>,
        // What casts into that sun (#1220).
        sun_shadow_layers: u32,
        eye: Vec3,
        scene_params: &SceneCullParams,
        meshlet_bg: &wgpu::BindGroup,
        track: crate::shadow::pages::raster::RasterTrack<'_>,
    ) {
        // 🔴 No sun does NOT skip the raster.
        let sun = sun.unwrap_or(Vec3::NEG_Y);
        let (Some(pool), Some(marker)) = (self.gpu_pool.as_ref(), self.page_marker.as_ref()) else {
            return;
        };
        // 🔴 One texel of simplification error, and NOT the camera's LOD target.
        let lod_target = 1.0_f32;
        let page_pool = marker.pool();
        let raster = self.page_raster.get_or_insert_with(|| {
            PageRasterizer::new(
                device,
                self.cull_pipelines.meshlet_bind_group_layout(),
                PageConfig::default(),
                ClipmapConfig::default(),
                settings.pool,
                crate::meshlet::DEFAULT_MAX_TRIANGLES as u32,
            )
        });
        // Groups are bounded by meshlets, and a slot is four bytes: the bound costs less than
        // threading the exact figure through a second call path would. Already stamped by
        // `bind_page_shadows`, which runs first.
        raster.set_frame(marker.life().frame);
        raster.set_softness(settings.softness);
        raster.set_march(settings.march);
        raster.set_geometry(settings.geometry);
        raster.set_bias(
            settings.bias.0,
            settings.bias.1,
            settings.bias.2,
            settings.bias.3,
        );
        // Before anything reads a stamp this frame: a world that was
        // replaced must not be sampled through the previous one's
        // pages (#971).
        raster.set_scene_epoch(settings.scene_epoch);
        raster.set_two_level(settings.two_level);
        let threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
        // 🔴 `group_capacity`, NOT `threads` (#1011).
        raster.ensure_capacity(
            device,
            threads,
            scene_params.group_capacity,
            scene_params.chunk_capacity,
        );
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
            sun_shadow_layers,
            self.lights.uploaded(),
            self.lights.light_buffer(),
            &self.moved_casters,
            lod_target,
            self.shadow_alpha.bind_group(),
            track,
        );
        // Idempotent, and this is the one call site that runs after every possible rebuild of
        // either side. The binding the SHADING reads is set by `bind_page_shadows` before the fused
        // pass; this one only makes sure a rebuilt atlas or table is picked up at all.
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
}
