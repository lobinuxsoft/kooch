//! One camera's frame: cull, compact, expand, draw.

use super::*;

impl PageRasterizer {
    /// Culls, compacts, expands and draws. Call **after** the marking
    /// pass: it reads the table marking filled.
    #[allow(clippy::too_many_arguments)]
    #[profiling::function]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        cull_pipelines: &MeshletCullPipelines,
        mesh_pool: &GpuGlobalMeshPool,
        scene: &MeshletScene,
        meshlet_bg: &wgpu::BindGroup,
        instances: &wgpu::Buffer,
        page_pool: &PagePool,
        scene_params: &SceneCullParams,
        view: u32,
        eye: Vec3,
        sun: Vec3,
        // What casts into the sun (#1220): every clipmap level culls with it.
        sun_shadow_layers: u32,
        // The lights as uploaded, CPU-side and in buffer order: a
        // lamp's cull needs its position and range HERE, and its slot
        // is its bucket.
        lamps: &[GpuLight],
        // The same lights on the GPU, for the expansion's cone test
        // and the depth pass's face placement.
        lights: &wgpu::Buffer,
        // World spheres of every caster that moved this frame — old
        // and new bounds alike — for the cache's invalidation pass.
        moved: &[[f32; 4]],
        lod_target: f32,
        // The transparent casters' coverage, when any casts by alpha this frame (#1224).
        alpha: Option<&wgpu::BindGroup>,
        track: RasterTrack<'_>,
    ) {
        let levels = self.clipmap.levels;
        let buckets = self.buckets();
        let light_count = lamps.len() as u32;
        let view = view.min(atlas_layers(self.pool) - 1);
        self.write_moved(device, queue, moved);
        self.write_uniform(queue, view, eye, sun, light_count);
        self.write_gens(queue, view, eye, sun, lamps);
        let uniform_offset = self.uniform_span(view).0 as u32;

        // 🔴 Cleared BEFORE anything writes it: a bucket whose cull does not run this frame — a
        // directional slot, a lamp past the cap — must read zero survivors, and an unwritten
        // storage buffer is not zero, it is whatever the allocator handed over.
        encoder.clear_buffer(&self.visible_counts, 0, Some(levels as u64 * 4));
        let cull_query = nested(track, "page lamp cull", encoder);
        // 1b. The lamps' shared hierarchical cull (#939) — Olsson et al.'s light/instance pre-pass,
        // then one group-coherent meshlet pass for every lamp at once.
        let lamp_slots = lamps.len().min(LAMP_CULLS as usize);
        if self.lamp_frame != Some(self.frame) {
            self.lamp_frame = Some(self.frame);
            profiling::scope!("cull: lamps");
            self.lamp_cull.record(
                device,
                queue,
                encoder,
                mesh_pool,
                instances,
                lights,
                &self.visible_counts,
                levels,
                lamp_slots as u32,
                scene_params.instance_count,
                scene_params.meshlets_per_mesh,
                scene_params.group_capacity,
                lod_target,
            );
        }
        close(track, cull_query, encoder);
        // The bucket uniforms for the expansion's lamp dispatches —
        // constant values, cheap to restate per view.
        for (slot, lamp) in lamps.iter().enumerate().take(lamp_slots) {
            if lamp.kind == LIGHT_KIND_DIRECTIONAL {
                continue;
            }
            let bucket = levels + slot as u32;
            queue.write_buffer(
                &self.levels,
                bucket as u64 * self.level_stride,
                bytemuck::bytes_of(&ExpandLevel {
                    level: bucket,
                    _pad: [0; 3],
                }),
            );
        }

        // Per view, all of it: the page list, the pair list and the dispatch arguments describe
        // THIS camera's clipmap and nothing else. The table, the pool and the atlas are the shared
        // things, and none of them is cleared here.
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);
        encoder.clear_buffer(&self.dirty, 0, Some(4));

        self.ensure_bound(device, page_pool, instances, &mesh_pool.meshlets, lights);
        let bound = self.bound.as_ref().expect("just built");

        let pages_query = nested(track, "page table", encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: invalidate and compact"),
                timestamp_writes: None,
            });
            // 1c. Invalidation, BEFORE the compaction reads the stamps:
            //     every page a moved caster reaches loses its content
            //     stamp and redraws like a fresh one.
            pass.set_pipeline(&self.invalidate);
            pass.set_bind_group(0, &bound.invalidate, &[uniform_offset]);
            pass.dispatch_workgroups(self.compact_threads(light_count).div_ceil(64), 1, 1);
            // 2. The flat table becomes a dense list, bucketed by
            //    octave — the dispatch covers exactly this camera's
            //    span, so the other view's pages are never walked.
            pass.set_pipeline(&self.compact);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(self.compact_threads(light_count).div_ceil(64), 1, 1);
            // 2b. The reader's jump table, after the compaction because
            //     "readable" means stamped and the stamp is what the
            //     compaction writes.
            let clipmap = levels * self.config.side(0).pow(2);
            pass.set_pipeline(&self.lod_offsets);
            pass.dispatch_workgroups(clipmap.div_ceil(64), 1, 1);
        }

        // 2b. The page pyramid, over the listing the compaction just wrote (#1022).
        {
            let sun_slot = super::mark::padded_lights(light_count);
            let stride = super::mark::stride(self.config, self.clipmap);
            let span = super::mark::span(self.config, self.clipmap, sun_slot + 1);
            let base = u32::try_from(view as u64 * span).unwrap_or(u32::MAX) + sun_slot * stride;
            self.pyramid
                .build(device, queue, encoder, page_pool.slots(), base);
        }

        close(track, pages_query, encoder);

        // 3. The culls, AFTER the page table is final.
        let cull_query = nested(track, "page cull", encoder);
        {
            profiling::scope!("cull: clipmap levels");
            for level in 0..levels {
                queue.write_buffer(
                    &self.levels,
                    level as u64 * self.level_stride,
                    bytemuck::bytes_of(&ExpandLevel {
                        level,
                        _pad: [0; 3],
                    }),
                );
                let clip = self.level_clip(level, eye, sun);
                let params = CullParams::shadow(
                    clip,
                    eye - sun.normalize_or(Vec3::NEG_Y) * SUN_SPAN,
                    scene_params.meshlets_per_mesh,
                )
                .with_orthographic_lod(
                    self.clipmap.extent(level),
                    self.config.virtual_size as f32,
                    lod_target.max(0.01),
                )
                .with_culling_mask(sun_shadow_layers);
                if self.two_level {
                    self.culls[level as usize].dispatch_scene_pool_atomic_chunked(
                        cull_pipelines,
                        device,
                        queue,
                        encoder,
                        mesh_pool,
                        scene,
                        &params,
                        scene_params,
                    );
                } else {
                    self.culls[level as usize].dispatch_scene_pool_atomic(
                        cull_pipelines,
                        device,
                        queue,
                        encoder,
                        mesh_pool,
                        scene,
                        &params,
                        scene_params,
                    );
                }
                // The expansion's dispatch size is pages times survivors,
                // and the survivor count only exists on the GPU.
                encoder.copy_buffer_to_buffer(
                    self.culls[level as usize].visible_count_buffer(),
                    0,
                    &self.visible_counts,
                    level as u64 * 4,
                    4,
                );
            }
        }
        close(track, cull_query, encoder);

        let expand_query = nested(track, "page expand", encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: expand"),
                timestamp_writes: None,
            });
            // 4a. The dispatch sizes, HERE and not with the compaction: they are a page count times
            // a survivor count, and the survivors only exist once the culls above have run. Sized
            // on the GPU because neither number ever reaches the CPU.
            pass.set_pipeline(&self.expand_args_pass);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(buckets.div_ceil(64), 1, 1);

            // 4b. Pairs. One indirect dispatch per level, sized by the dispatch above rather than
            // by a CPU guess. The only thing that changes between levels is two dynamic offsets and
            // the visible list — no bind group is built here.
            pass.set_pipeline(&self.expand);
            pass.set_bind_group(1, &bound.descriptors, &[]);
            pass.set_bind_group(3, &bound.instances, &[]);
            // The sun's buckets against its level culls, then each lamp's bucket against ITS OWN
            // cull. `bound.visible` holds them in the same order — levels first, lamp slots after —
            // so the bucket index is the bind-group index throughout.
            for level in 0..levels {
                pass.set_bind_group(
                    0,
                    &bound.expand,
                    &[uniform_offset, level * self.level_stride as u32],
                );
                pass.set_bind_group(2, &bound.visible[level as usize], &[]);
                pass.dispatch_workgroups_indirect(&self.expand_args, level as u64 * 12);
            }
            // The lamps: ONE bind group — the shared survivor arena — and a slot's slice is
            // arithmetic inside the shader, so the only thing that changes per bucket is the
            // dynamic offset.
            pass.set_bind_group(2, &bound.lamp_visible, &[]);
            for (slot, lamp) in lamps.iter().enumerate().take(lamp_slots) {
                if lamp.kind == LIGHT_KIND_DIRECTIONAL {
                    continue;
                }
                let bucket = levels + slot as u32;
                pass.set_bind_group(
                    0,
                    &bound.expand,
                    &[uniform_offset, bucket * self.level_stride as u32],
                );
                pass.dispatch_workgroups_indirect(&self.expand_args, bucket as u64 * 12);
            }

            // 4. One draw for the whole clipmap, so its instance count
            //    is the whole pair list.
            pass.set_pipeline(&self.draw_args_pass);
            pass.set_bind_group(0, &bound.compact, &[uniform_offset]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        close(track, expand_query, encoder);

        let depth_query = nested(track, "page depth", encoder);
        // 🔴 One pass per LAYER of this view (#1016). A render pass attaches a single layer, so a
        // view spread across several needs one each — and the draws inside test their page against
        // the layer they are in, because a page's rect is the same texels of every layer.
        for local in 0..self.pool.layers_per_view() {
            let layer = self.layer_of(view, local);
            let layer_offset = self.layer_span(layer).0 as u32;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pages: depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    // 🔴 THIS camera's layer, LOADED — never cleared. The cache is the layer's
                    // content: a resident page whose stamp still matches keeps last frame's depth,
                    // and only the dirty pages' rects are wiped, by the quad draw below.
                    view: &self.layers[layer as usize],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The per-page clear: one quad per dirty page at far depth
            // (reversed-Z 0 — "nothing between here and the light"),
            // depth test Always. Then the pairs draw over clean rects.
            pass.set_pipeline(&self.page_clear);
            pass.set_bind_group(0, &bound.clear, &[layer_offset]);
            pass.draw_indirect(&self.draw_args, 16);
            match alpha {
                Some(alpha) => {
                    pass.set_pipeline(&self.depth_alpha);
                    pass.set_bind_group(3, alpha, &[]);
                }
                None => pass.set_pipeline(&self.depth),
            }
            pass.set_bind_group(0, &bound.depth, &[layer_offset]);
            pass.set_bind_group(1, meshlet_bg, &[]);
            pass.set_bind_group(2, &bound.instances, &[]);
            pass.draw_indirect(&self.draw_args, 0);
        }
        close(track, depth_query, encoder);

        // The survivor counts, brought home alongside the page counts so
        // the expansion's cost can be read as the product it is. See
        // `count_slots`.
        encoder.copy_buffer_to_buffer(
            &self.visible_counts,
            0,
            &self.counts,
            (self.buckets() as u64 + 5) * 4,
            self.buckets() as u64 * 4,
        );
        self.readback.record(encoder, &self.counts, view);
    }
}
