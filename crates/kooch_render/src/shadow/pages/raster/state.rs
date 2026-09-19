//! What a frame writes before it draws: generations, moved casters, uniforms and bind groups.

use super::*;

impl PageRasterizer {
    /// Threads the compaction needs: one per entry of one view's span.
    pub(super) fn compact_threads(&self, light_count: u32) -> u32 {
        let slots = super::mark::padded_lights(light_count) + 1;
        u32::try_from(super::mark::span(self.config, self.clipmap, slots)).unwrap_or(u32::MAX)
    }

    /// One generation per bucket owner, for the cache gate.
    pub(super) fn write_gens(
        &self,
        queue: &wgpu::Queue,
        view: u32,
        eye: Vec3,
        sun: Vec3,
        lamps: &[GpuLight],
    ) {
        let gens = self.gens_for(eye, sun, lamps);
        queue.write_buffer(
            &self.gens,
            view as u64 * self.buckets() as u64 * 4,
            bytemuck::cast_slice(&gens),
        );
    }

    /// [`Self::write_gens`] without the upload — the arithmetic alone,
    /// so a test can ask what a camera move does to the cache without a
    /// queue to write into.
    pub(crate) fn gens_for(&self, eye: Vec3, sun: Vec3, lamps: &[GpuLight]) -> Vec<u32> {
        let levels = self.clipmap.levels as usize;
        let buckets = self.buckets() as usize;
        let mut gens = vec![0u32; buckets];
        let side = self.config.side(0) as f32;
        gens[..levels].copy_from_slice(&sun_gens(self.clipmap, side, self.scene_gen, eye, sun));
        for slot in 0..LAMP_CULLS as usize {
            let mut h = FNV_SEED;
            if let Some(lamp) = lamps.get(slot) {
                for word in [
                    lamp.position[0].to_bits(),
                    lamp.position[1].to_bits(),
                    lamp.position[2].to_bits(),
                    lamp.direction[0].to_bits(),
                    lamp.direction[1].to_bits(),
                    lamp.direction[2].to_bits(),
                    lamp.range.to_bits(),
                    lamp.kind,
                    lamp.spot_scale.to_bits(),
                    lamp.spot_offset.to_bits(),
                ] {
                    h = fnv(h, word);
                }
            }
            h = fnv(h, self.scene_gen);
            gens[levels + slot] = h | 1;
        }
        gens
    }

    /// Uploads the frame's moved-caster spheres — once, not per view —
    /// or, past the buffer, bumps the scene generation so everything
    /// redraws instead of something staying silently stale.
    pub(super) fn write_moved(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        moved: &[[f32; 4]],
    ) {
        if self.moved_frame == Some(self.frame) {
            return;
        }
        self.moved_frame = Some(self.frame);
        self.ensure_moved(device, moved.len());
        if moved.len() > self.moved_capacity as usize {
            // 🔴 Said out loud, because the fallback is silent and total: past the cap the scene
            // generation bumps, which voids EVERY page every frame it happens.
            if !self.flooded {
                self.flooded = true;
                tracing::warn!(
                    target: "kooch_render::shadow",
                    moved = moved.len(),
                    ceiling = MOVED_CEILING,
                    "the moved-caster list is past its ceiling; every page redraws while it is",
                );
            }
            self.scene_gen = self.scene_gen.wrapping_add(1);
            queue.write_buffer(&self.moved, 0, bytemuck::bytes_of(&[0.0f32; 4]));
            return;
        }
        if self.flooded {
            self.flooded = false;
            tracing::info!(
                target: "kooch_render::shadow",
                moved = moved.len(),
                "the moved-caster list fits again; the page cache is live",
            );
        }
        let mut data = Vec::with_capacity(1 + moved.len());
        data.push([moved.len() as f32, 0.0, 0.0, 0.0]);
        data.extend_from_slice(moved);
        queue.write_buffer(&self.moved, 0, bytemuck::cast_slice(&data));
    }

    /// Grows the moved-caster list to what this frame actually moved.
    pub(super) fn ensure_moved(&mut self, device: &wgpu::Device, spheres: usize) {
        let wanted = u32::try_from(spheres)
            .unwrap_or(u32::MAX)
            .min(MOVED_CEILING);
        if wanted <= self.moved_capacity {
            return;
        }
        self.moved_capacity = wanted;
        self.moved = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("page_raster_moved"),
            size: moved_bytes(self.moved_capacity),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // 🔴 The bind groups hold the OLD buffer. `BoundKeys` carries
        // `moved` for exactly this, and a cached group pointing at a
        // freed buffer is a validation error every frame.
        self.bound = None;
    }

    /// The uniform every raster pass reads. Written once a frame, before any of them. 🔴 One per
    /// LAYER of the view, not one per view (#1016).
    pub(super) fn write_uniform(
        &self,
        queue: &wgpu::Queue,
        view: u32,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
    ) {
        for local in 0..self.pool.layers_per_view() {
            self.write_layer_uniform(queue, view, local, eye, sun, lights);
        }
    }

    pub(super) fn write_layer_uniform(
        &self,
        queue: &wgpu::Queue,
        view: u32,
        local: u32,
        eye: Vec3,
        sun: Vec3,
        lights: u32,
    ) {
        let d = sun.normalize_or(Vec3::NEG_Y);
        // The sun's region starts after the PADDED light slots, the way
        // marking lays the space out — see `padded_lights` for why the
        // padding and not the raw count.
        let sun_slot = super::mark::padded_lights(lights);
        let stride = super::mark::stride(self.config, self.clipmap);
        let view_span = super::mark::span(self.config, self.clipmap, sun_slot + 1);
        let layer = self.layer_of(view, local);
        queue.write_buffer(
            &self.uniform,
            self.layer_span(layer).0,
            bytemuck::bytes_of(&RasterUniform {
                space: [
                    stride,
                    self.config.local_face_pages(),
                    self.config.side(0),
                    sun_slot,
                ],
                views: [
                    view.min(atlas_layers(self.pool) - 1),
                    u32::try_from(view_span).unwrap_or(u32::MAX),
                    self.pool.slice(),
                    // 🔴 Only the age debug view reads this. A page's age is a difference against
                    // the current frame, and the shading pass has no other way to know what frame
                    // it is in.
                    self.frame,
                ],
                pool: [
                    u32::try_from(view_span * atlas_layers(self.pool) as u64).unwrap_or(u32::MAX),
                    self.pool.total(),
                    self.pool.per_row(),
                    self.config.page,
                ],
                chain: [
                    self.clipmap.levels,
                    PAIR_CAPACITY,
                    bucket(self.pool),
                    // 🔴 Triangles a MESHLET may hold, which is the fixed vertex count the indirect
                    // draw issues — `max_triangles_per_meshlet * 3`, the same figure
                    // `MeshletCull::new` documents for the cascades' draw.
                    self.triangles,
                ],
                world: [
                    self.clipmap.base,
                    SUN_SPAN,
                    // The side of ONE LAYER, which is what a page's clip
                    // position is placed inside.
                    (self.pool.per_row() * self.config.page) as f32,
                    // The readers' PCF footprint width (#941). In the
                    // raster's own uniform because the shading binds
                    // this exact buffer — one write serves both.
                    self.softness.max(1) as f32,
                ],
                eye: [eye.x, eye.y, eye.z, 0.0],
                sun: [d.x, d.y, d.z, 1.0],
                // Same reason as the softness above: the shading binds
                // this exact buffer, so one write serves both.
                bias: self.bias,
                layer: [layer, view, u32::from(self.march), u32::from(self.geometry)],
            }),
        );
    }

    /// The atlas layer a view's `local`-th layer is, globally.
    pub fn layer_of(&self, view: u32, local: u32) -> u32 {
        let per_view = self.pool.layers_per_view();
        (view.min(self.pool.view_count() - 1) * per_view + local.min(per_view - 1))
            .min(atlas_layers(self.pool) - 1)
    }

    /// The uniform slice a LAYER reads, for the pass attached to it.
    pub fn layer_span(&self, layer: u32) -> (u64, u64) {
        (
            self.uniform_stride * layer.min(atlas_layers(self.pool) - 1) as u64,
            std::mem::size_of::<RasterUniform>() as u64,
        )
    }

    /// The table becomes a dense list, bucketed by level, and the expansion's dispatch sizes are
    /// computed from it.
    #[allow(clippy::too_many_arguments)]
    #[profiling::function]
    pub fn record_compaction(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        page_pool: &PagePool,
        view: u32,
        eye: Vec3,
        sun: Vec3,
        lamps: &[GpuLight],
    ) {
        self.write_uniform(queue, view, eye, sun, lamps.len() as u32);
        self.write_gens(
            queue,
            view.min(atlas_layers(self.pool) - 1),
            eye,
            sun,
            lamps,
        );
        encoder.clear_buffer(&self.counts, 0, None);
        encoder.clear_buffer(&self.expand_args, 0, None);
        encoder.clear_buffer(&self.draw_args, 0, None);
        encoder.clear_buffer(&self.dirty, 0, Some(4));
        let bind_group = self.compact_bind_group(device, page_pool);
        let offset = [self.uniform_span(view).0 as u32];
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow pages: compact"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.compact);
        pass.set_bind_group(0, &bind_group, &offset);
        pass.dispatch_workgroups(self.compact_threads(lamps.len() as u32).div_ceil(64), 1, 1);
        pass.set_pipeline(&self.expand_args_pass);
        pass.dispatch_workgroups(self.buckets().div_ceil(64), 1, 1);
        // Without a pair list this only fixes the vertex count, which is
        // exactly the half worth asserting on without a scene.
        pass.set_pipeline(&self.draw_args_pass);
        pass.dispatch_workgroups(1, 1, 1);
    }

    pub(super) fn compact_bind_group(
        &self,
        device: &wgpu::Device,
        page_pool: &PagePool,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_compact_bg"),
            layout: &self.compact_bgl,
            entries: &[
                self.uniform_entry(0),
                entry(2, page_pool.slots()),
                entry(3, &self.page_list),
                entry(4, &self.counts),
                entry(5, &self.expand_args),
                entry(6, &self.visible_counts),
                entry(7, &self.draw_args),
                entry(8, &self.gens),
                entry(9, &self.dirty),
            ],
        })
    }

    /// The uniform, bound as ONE camera's slice. The slice that gets
    /// read is picked by the dynamic offset at `set_bind_group` time,
    /// so the same group serves every camera.
    pub(super) fn uniform_entry(&self, binding: u32) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.uniform,
                offset: 0,
                size: std::num::NonZeroU64::new(std::mem::size_of::<RasterUniform>() as u64),
            }),
        }
    }

    /// Builds every bind group the passes need, and only when one of the buffers behind them has
    /// actually been replaced.
    pub(super) fn ensure_bound(
        &mut self,
        device: &wgpu::Device,
        page_pool: &PagePool,
        instances: &wgpu::Buffer,
        descriptors: &wgpu::Buffer,
        lights: &wgpu::Buffer,
    ) {
        let keys = BoundKeys {
            slots: page_pool.slots().clone(),
            instances: instances.clone(),
            descriptors: descriptors.clone(),
            lights: lights.clone(),
            visible: self
                .culls
                .iter()
                .map(|c| c.visible_meshlets_buffer().clone())
                .collect(),
            lamp_survivors: self.lamp_cull.survivors().clone(),
            moved: self.moved.clone(),
        };
        if self.bound.as_ref().is_some_and(|b| b.keys == keys) {
            return;
        }
        let storage = |label: &str, buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.storage_bgl,
                entries: &[entry(0, buffer)],
            })
        };
        self.bound = Some(Bound {
            compact: self.compact_bind_group(device, page_pool),
            expand: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_expand_bg"),
                layout: &self.expand_bgl,
                entries: &[
                    self.uniform_entry(0),
                    entry(1, &self.page_list),
                    entry(2, &self.counts),
                    entry(3, &self.pairs),
                    entry(4, &self.visible_counts),
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.levels,
                            offset: 0,
                            size: std::num::NonZeroU64::new(
                                std::mem::size_of::<ExpandLevel>() as u64
                            ),
                        }),
                    },
                    entry(6, lights),
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(self.pyramid.view()),
                    },
                ],
            }),
            depth: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_depth_bg"),
                layout: &self.depth_bgl,
                entries: &[
                    self.uniform_entry(0),
                    entry(1, lights),
                    entry(2, &self.pairs),
                ],
            }),
            invalidate: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_invalidate_bg"),
                layout: &self.invalidate_bgl,
                entries: &[
                    self.uniform_entry(0),
                    entry(2, page_pool.slots()),
                    entry(10, &self.moved),
                    entry(11, lights),
                ],
            }),
            clear: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_clear_bg"),
                layout: &self.clear_bgl,
                entries: &[self.uniform_entry(0), entry(3, &self.dirty)],
            }),
            visible: self
                .culls
                .iter()
                .map(|c| storage("page_expand_visible_bg", c.visible_meshlets_buffer()))
                .collect(),
            lamp_visible: storage("page_expand_lamp_bg", self.lamp_cull.survivors()),
            instances: storage("page_raster_instances_bg", instances),
            descriptors: storage("page_raster_descriptors_bg", descriptors),
            keys,
        });
    }

    /// The `(page, slot, meshlet)` pairs the expansion emitted.
    pub fn pairs_buffer(&self) -> &wgpu::Buffer {
        &self.pairs
    }

    /// The compacted pages, for whoever reads them back.
    /// COPY_SRC so a test can read it.
    pub fn page_list_buffer(&self) -> &wgpu::Buffer {
        &self.page_list
    }

    /// Grows every clipmap level's cull to the scene. The lamps'
    /// shared arena sizes itself at record time, when the frame's
    /// active light count is known.
    pub fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        meshlets: u32,
        groups: u32,
        chunks: u32,
    ) {
        for cull in &mut self.culls {
            // 🔴 FIRST, and that ordering is the whole of it: nothing reads these culls' reject
            // buffer — the debug overlay is wired to the camera's — and `ensure_capacity` decides
            // its size from this flag.
            cull.set_rejects(false);
            cull.ensure_capacity(device, meshlets.max(1));
            cull.ensure_group_capacity(device, groups.max(1));
            cull.ensure_chunk_capacity(device, chunks.max(1));
        }
    }

    /// Chooses the cull's dispatch shape for every clipmap level (#1002).
    pub fn set_two_level(&mut self, two_level: bool) {
        self.two_level = two_level;
    }

    /// The clipmap level's orthographic clip-from-world.
    pub(super) fn level_clip(&self, level: u32, eye: Vec3, sun: Vec3) -> Mat4 {
        level_clip(self.clipmap, self.config.side(0), level, eye, sun)
    }
}
