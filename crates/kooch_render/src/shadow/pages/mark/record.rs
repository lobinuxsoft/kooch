//! The marking dispatch, recorded once per camera.

use super::*;

impl PageMarker {
    /// Records the dispatch, sizing the mark buffer if the scene grew.
    #[allow(clippy::too_many_arguments)]
    #[profiling::function]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        lights: &GpuLights,
        depth: &wgpu::TextureView,
        world_from_clip: Mat4,
        eye: Vec3,
        sun: Option<Vec3>,
        viewport: (u32, u32),
        // Which camera this dispatch is for. Decides the slice of the
        // pool it allocates from, its region of the mark bitmap and the
        // high part of every page id it writes.
        view: u32,
        rate: u32,
        // Shadow texels per screen pixel, as a percentage.
        density: u32,
        paint: Paint<'_>,
    ) {
        let count = lights.light_count().max(1);
        // One slot past the lights, for the sun: it is not in the grid — it has no position to
        // cluster — so it gets a region of its own at the tail rather than a light index.
        let padded = padded_lights(count);
        let slots = padded + 1;
        let views = self.pool.config().view_count();
        let view = view.min(views - 1);
        if (slots, views) != self.capacity {
            self.marks = marks_buffer(device, self.config, self.clipmap, slots, views);
            self.rank = rank_buffer(device, views);
            // 🔴 The TABLE goes with them, and it did not (#973).
            self.pool.clear(encoder);
            self.life.rebuilt = true;
            tracing::info!(
                target: "kooch_render::shadow",
                slots,
                views,
                "the page table was re-addressed; clearing it and the free list",
            );
            self.capacity = (slots, views);
        }
        // The flat table is one entry per addressable page, so its size follows the address space.
        // A growth replaces the buffers — every entry gone — so the next frame is flagged as a
        // rebuild and `age_view` evicts the nothing that is left, keeping the allocator honest.
        let view_span = span(self.config, self.clipmap, slots);
        let entries = u32::try_from(view_span * views as u64).unwrap_or(u32::MAX);
        if self.pool.ensure_entries(device, entries) {
            self.life.rebuilt = true;
        }

        // 🔴 Painting forces one thread per pixel. At any coarser rate the view would be a grid of
        // dots over an unpainted frame, which reads as "the pass is broken" rather than as "you
        // asked for one sample in sixteen".
        let rate = if paint.on {
            1
        } else {
            rate.clamp(RATE_RANGE.0, RATE_RANGE.1)
        };
        queue.write_buffer(
            &self.view,
            0,
            bytemuck::bytes_of(&PageMarkView {
                world_from_clip: world_from_clip.to_cols_array_2d(),
                eye_and_base: [eye.x, eye.y, eye.z, self.clipmap.base],
                sun: sun
                    .map(|d| {
                        let d = d.normalize_or_zero();
                        [d.x, d.y, d.z, 1.0]
                    })
                    .unwrap_or([0.0, -1.0, 0.0, 0.0]),
                chain: [
                    self.config.page,
                    self.config.virtual_size,
                    self.config.levels(),
                    self.clipmap.levels,
                ],
                strides: [
                    self.config.side(0),
                    self.config.local_face_pages(),
                    self.stride(),
                    count,
                ],
                // 🔴 `sampling.y` is the SUN'S SLOT — the padded light
                // count, not the real one. The real count stays in
                // `strides.w` for the marking loop's guard.
                sampling: [rate, padded, u32::from(paint.on), view],

                pool: [
                    self.pool.entries(),
                    self.pool.config().total(),
                    self.pool.config().per_row(),
                    // 🔴 A VIEW's pages, not a layer's (#1016). Every reader of this word — the free
                    // list's stride, the bump's ceiling, the seat budget — asks "how many pages
                    // does this camera own".
                    self.pool.config().slots(),
                ],
                // `words()` leaves the fourth word at zero; the sun's half-span rides it (#949).
                life: {
                    let mut life = self.life.words();
                    life[3] = super::raster::SUN_SPAN.to_bits();
                    life
                },
                // How many output pixels one depth pixel covers, per
                // axis. 1 when nothing is upscaling.
                paint: [
                    paint.size.0 as f32 / viewport.0.max(1) as f32,
                    paint.size.1 as f32 / viewport.1.max(1) as f32,
                    paint.size.0 as f32,
                    paint.size.1 as f32,
                ],
                // The reciprocal, because the shader scales the world
                // size a pixel may ask a texel to match.
                density: [
                    100.0 / density.clamp(1, 400) as f32,
                    // The coverage gate (#944), in projected pixels.
                    self.coverage as f32,
                    // 🔴 Non-zero moves the per-LIGHT loop off the pixel and onto the froxel (#952).
                    if self.cluster { 1.0 } else { 0.0 },
                    // The distance gate, in multiples of a light's own
                    // range. See `light_out_of_reach`.
                    self.reach as f32,
                ],
                halo: [self.halo, 0.0, 0.0, 0.0],
            }),
        );

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_mark_bind_group"),
            layout: &self.layout,
            entries: &[
                buffer_entry(0, lights.clusters().view_uniform()),
                buffer_entry(1, lights.clusters().cells()),
                buffer_entry(2, lights.clusters().indices()),
                buffer_entry(3, lights.light_buffer()),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                buffer_entry(5, &self.view),
                buffer_entry(6, &self.marks),
                buffer_entry(7, &self.counters),
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(paint.target),
                },
                buffer_entry(9, &self.rank),
                buffer_entry(10, self.pool.slots()),
                buffer_entry(11, self.pool.alloc()),
            ],
        });

        // 🔴 This VIEW'S bits, not the whole bitmap. A view's pages are
        // a contiguous run — that is what `stride` is rounded to a
        // multiple of 32 for — so the reset is an offset clear.
        let words = view_span.div_ceil(32) * 4;
        encoder.clear_buffer(&self.marks, words * view as u64, Some(words));
        // Only the demand histogram: the plan's words are stored anew every frame before anything
        // reads them, and the bias and its patience (#943) PERSIST — they are what one frame
        // teaches the next.
        let run = view as u64 * RANK_WORDS * 4;
        // 🔴 Two ranges, not one. The bias and the patience (#943) are
        // PERSISTENT and sit between the plan and the bitmap, so a single
        // clear across the run would wipe what the pressure loop learned.
        encoder.clear_buffer(&self.rank, run, Some(32 * 4));
        encoder.clear_buffer(
            &self.rank,
            run + RANK_OCCUPANCY * 4,
            Some((OCCUPANCY_WORDS + DEPTH_WORDS) * 4),
        );
        // Every counter here is a per-view quantity now, the pool's
        // claims included: a view allocates out of its own slice.
        encoder.clear_buffer(&self.counters, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow pages: mark"),
                timestamp_writes: None,
            });
            // The table is flat and a view's entries are contiguous, so
            // the ageing walks exactly this view's span — the other
            // camera's pages are outside the dispatch.
            pass.set_pipeline(&self.clear);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                u32::try_from(view_span)
                    .unwrap_or(u32::MAX)
                    .div_ceil(GROUP * GROUP),
                1,
                1,
            );
            pass.set_pipeline(&self.pipeline);
            let threads = (viewport.0.div_ceil(rate), viewport.1.div_ceil(rate));
            pass.dispatch_workgroups(threads.0.div_ceil(GROUP), threads.1.div_ceil(GROUP), 1);
            // Olsson §III (#952): the same marking over OCCUPIED FROXELS rather than pixels — 199
            // of them against 163 864 covered pixels in `many_lights`.
            if self.cluster {
                pass.set_pipeline(&self.froxel_mark);
                pass.dispatch_workgroups(OCCUPANCY_MAX.div_ceil(GROUP * GROUP), 1, 1);
            }
            // The seat passes (#942), in an order that is the algorithm: rank the demand, clear
            // what the plan does not fund, seat what it does. Dispatch boundaries are the barriers
            // between them.
            let entries = u32::try_from(view_span)
                .unwrap_or(u32::MAX)
                .div_ceil(GROUP * GROUP);
            pass.set_pipeline(&self.plan);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.preempt);
            pass.dispatch_workgroups(entries, 1, 1);
            pass.set_pipeline(&self.adopt);
            pass.dispatch_workgroups(entries, 1, 1);
            pass.set_pipeline(&self.bias);
            pass.dispatch_workgroups(1, 1, 1);
            // After the marking has filled the bitmap; one group covers
            // `OCCUPANCY_WORDS`.
            pass.set_pipeline(&self.census);
            pass.dispatch_workgroups(1, 1, 1);
        }
        // Kept for `record_paint`, which runs after the shading and needs
        // every one of these bindings — the depth, the view uniform and
        // the colour target included.
        self.bound = Some(bind_group);
        self.pending = self.readback.record(
            encoder,
            &self.counters,
            Label {
                size: viewport,
                view,
                capacity: self.pool.config().slots(),
            },
        );
    }
}
