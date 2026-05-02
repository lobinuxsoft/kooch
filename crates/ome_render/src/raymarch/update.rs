//! ECS → GPU upload: camera and SDF scene data.
//!
//! Collects all visible SDF shape entities into a primitives storage
//! buffer (sorted by entity index for stable ordering), groups them by
//! CSG role (add / intersect / subtract) according to their optional
//! `SdfBlend`, computes per-primitive world-space AABBs (inflated by
//! the per-role smooth-blend k_max so the BVH cull stays conservative),
//! and kicks a GPU LBVH build whenever the scene state changes.
//!
//! The shader iterates the BVH stack-based and accumulates per-role
//! distances inline — no postfix CSG token stream is uploaded any
//! more (PR-4 of #115).

use glam::{Mat4, Vec3, Vec4};
use ome_bvh::{
    Aabb, IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD, ROLE_RAYMARCH_INT, ROLE_RAYMARCH_SUB,
};
use ome_core::coord::ActiveOrigin;
use ome_ecs::PerspectiveCamera;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::sdf_blend::{MODE_SMOOTH_INTERSECTION, MODE_SMOOTH_SUBTRACTION};

use super::aabb::primitive_aabb;
use super::collect::{CollectedRow, collect_all_visible_sdfs};
use super::instance::{RaymarchPayload, SceneMeta, SdfPrimitive};
use super::renderer::RayMarchRenderer;

impl RayMarchRenderer {
    /// Uploads the active camera (from ECS) to the GPU.
    ///
    /// Picks the first `active` `PerspectiveCamera` paired with a
    /// `GlobalTransform` by highest `priority`. Returns `true` when a
    /// camera was found.
    pub fn update_camera(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &ome_core::resource::Resources,
        aspect: f32,
        screen_height: u32,
    ) -> bool {
        let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
        let mut best: Option<(i32, PerspectiveCamera, Mat4)> = None;
        query.for_each(|(cam, gt)| {
            if !cam.active {
                return;
            }
            let better = match &best {
                Some((p, _, _)) => cam.priority > *p,
                None => true,
            };
            if better {
                best = Some((cam.priority, *cam, gt.matrix));
            }
        });
        drop(query);

        let Some((_, cam, world_matrix)) = best else {
            return false;
        };

        let view = world_matrix.inverse();
        let projection = Mat4::perspective_rh(
            cam.fov.to_radians(),
            aspect.max(0.001),
            cam.near.max(0.001),
            cam.far.max(cam.near + 0.001),
        );
        let (_, _, translation) = world_matrix.to_scale_rotation_translation();

        // Plumb ActiveOrigin: the camera's `translation` is f32 in the
        // simulation frame anchored at ActiveOrigin. Composing the two
        // gives the absolute universe position — emitted at TRACE level
        // so debug HUDs / future per-frame world-space passes have an
        // end-to-end consumer of the coord system without forcing the
        // shader to gain new uniforms before there's a real use case.
        if let Some(active_origin) = resources.get::<ActiveOrigin>() {
            let universe_pos = active_origin
                .coord()
                .translated(translation.as_dvec3());
            tracing::trace!(
                target: "ome_render::raymarch",
                sector = ?universe_pos.sector,
                offset = ?universe_pos.offset,
                "camera universe position"
            );
        }

        // PR-5 (epic #370): per-pixel cone half-angle at unit `t`.
        // `tan(fov_y / 2.0) * 2.0 / screen_height` — vertical-axis
        // formulation matches the projection's vertical FOV. The
        // fragment shader's `pick_cascade` multiplies this by
        // `length(p - camera.position)` to compare against the
        // cascade voxel pitch table.
        let pixel_cone_angle =
            (cam.fov.to_radians() * 0.5).tan() * 2.0 / screen_height.max(1) as f32;
        let uniforms = super::instance::CameraUniforms {
            view: view.to_cols_array_2d(),
            projection: projection.to_cols_array_2d(),
            inverse_view: view.inverse().to_cols_array_2d(),
            inverse_projection: projection.inverse().to_cols_array_2d(),
            position: translation.to_array(),
            pixel_cone_angle,
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniforms));
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
        // PR-4 (epic #370): cache the camera world position so
        // `update_scene` can centre the GDF cascade on it.
        self.last_camera_pos = Vec3::new(translation.x, translation.y, translation.z);
        true
    }

    /// Uploads every visible SDF primitive entity, kicks a BVH rebuild
    /// when the scene changed, and refreshes the scene bind group.
    ///
    /// **Caller contract:** [`Self::apply_streaming_delta`] must run
    /// before this method on every frame the renderer ticks — that
    /// keeps `pending_loads` from growing unbounded when no ECS-side
    /// SDFs are visible (the streaming chunks themselves are reason
    /// enough to render). The viewport calls `apply_streaming_delta`
    /// **before** the `has_sdf` gate so streaming-only scenes light up.
    ///
    /// `skip_internal_sky = true` tells the fragment shader to discard on
    /// ray miss instead of drawing its internal gradient — use this when a
    /// separate sky pass ran before the raymarch and already filled the
    /// background.
    pub fn update_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut ome_core::resource::Resources,
        sky_top: Vec4,
        sky_bottom: Vec4,
        skip_internal_sky: bool,
    ) {
        let mut tagged: Vec<CollectedRow> = collect_all_visible_sdfs(resources);
        // Stable ordering: identical ECS state always serialises to the
        // same primitive array + the same BVH input (and therefore the
        // same byte-identical render output frame-to-frame).
        tagged.sort_by_key(|(idx, _, _)| *idx);

        // First pass: per-role smoothness maxima + presence flags. The
        // BVH cull inflates each leaf's AABB by its role's k_max so a
        // smooth blend's tail can't sneak past the boundary.
        let mut k_add_max = 0.0f32;
        let mut k_int_max = 0.0f32;
        let mut k_sub_max = 0.0f32;
        let mut has_intersects = false;
        let mut has_subs = false;
        for (_, _, blend) in &tagged {
            match blend.mode {
                MODE_SMOOTH_INTERSECTION => {
                    has_intersects = true;
                    k_int_max = k_int_max.max(blend.smoothness);
                }
                MODE_SMOOTH_SUBTRACTION => {
                    has_subs = true;
                    k_sub_max = k_sub_max.max(blend.smoothness);
                }
                _ => {
                    k_add_max = k_add_max.max(blend.smoothness);
                }
            }
        }

        // Second pass: build per-leaf metadata + BVH input items.
        let mut primitives: Vec<SdfPrimitive> = Vec::with_capacity(tagged.len());
        let mut leaf_aabbs: Vec<LeafAabb> = Vec::with_capacity(tagged.len());
        let mut raymarch_payloads: Vec<RaymarchPayload> = Vec::with_capacity(tagged.len());
        let mut bvh_items: Vec<(u32, Aabb)> = Vec::with_capacity(tagged.len());
        for (entity_idx, mut prim, blend) in tagged {
            let prim_idx = primitives.len() as u32;
            let role_bits = match blend.mode {
                MODE_SMOOTH_INTERSECTION => ROLE_RAYMARCH_INT,
                MODE_SMOOTH_SUBTRACTION => ROLE_RAYMARCH_SUB,
                _ => ROLE_RAYMARCH_ADD,
            };
            let inflation = match role_bits {
                ROLE_RAYMARCH_INT => k_int_max,
                ROLE_RAYMARCH_SUB => k_sub_max,
                _ => k_add_max,
            };
            let aabb = primitive_aabb(&prim, inflation);
            bvh_items.push((prim_idx, aabb));
            leaf_aabbs.push(LeafAabb {
                aabb_min: aabb.min.to_array(),
                flags: IS_RAYMARCH | role_bits,
                aabb_max: aabb.max.to_array(),
                entity_id: entity_idx,
            });
            raymarch_payloads.push(RaymarchPayload {
                smoothness: blend.smoothness,
            });
            // Pool path consumes per-primitive smoothness inline from
            // the `SdfPrimitive` instead of via a parallel buffer.
            prim.smoothness = blend.smoothness;
            primitives.push(prim);
        }

        // Discard the now-unused `bvh_items` + `raymarch_payloads`:
        // the pool path consumes per-primitive smoothness inline from
        // `prim.smoothness`, and the BVH input pairs are derived
        // inside `OmeAccel::insert_chunk` from the leaf AABB list.
        let _ = bvh_items;
        let _ = raymarch_payloads;

        // Drive the OmeAccel pool with one chunk holding every
        // visible ECS primitive. Streaming chunks coexist via the
        // separate `apply_streaming_delta` → `insert_streaming_chunk`
        // path; their keys are bit-63-flagged so the legacy
        // `SINGLE_CHUNK_KEY = 0` slot stays disjoint from streaming.
        let envelope = k_add_max.max(k_int_max).max(k_sub_max);
        let primitive_count = primitives.len() as u32;
        if let Err(e) =
            self.bvh_state
                .update_single_chunk(queue, &leaf_aabbs, &primitives, envelope)
        {
            tracing::warn!("OmeAccel single-chunk update failed: {e}; pool unchanged");
        }
        // Single tick per frame after every pool mutation has landed
        // (apply_streaming_delta from the viewport, update_single_chunk
        // above). One TLAS rebuild + uniforms upload regardless of how
        // many inserts/removes fired this frame. PR-1 of epic #370
        // threaded an encoder into `update_gpu` for the GPU-driven TLAS
        // rebuild compute pass, so `tick_uniforms` now opens an ad-hoc
        // encoder + submits per `update_scene` call until a higher-level
        // frame batch lands.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_render::update_scene_tlas_encoder"),
        });
        self.bvh_state
            .tick_uniforms(queue, &mut encoder, k_int_max, k_sub_max);
        // PR-5 (epic #370): round-robin populate. The scheduler
        // returns the cascade indices that need a redispatch this
        // frame (cascade 0 every frame, cascade `c` every `2^c`
        // frames in steady state, plus on-demand triggers for
        // dirtied chunks and camera drift). All dispatches land in
        // the same encoder, so the fragment shader sees a coherent
        // multi-cascade SDF in the next render pass.
        let cascades = self.gdf_scheduler.cascades_to_update(self.last_camera_pos);
        for cascade_idx in cascades {
            self.gdf_state.dispatch_populate_cascade(
                &mut encoder,
                queue,
                cascade_idx as usize,
                self.last_camera_pos,
            );
        }
        queue.submit(std::iter::once(encoder.finish()));

        // `SceneMeta` keeps the legacy field layout (uniform buffer
        // contract). Pool path only consumes `skip_internal_sky` +
        // `sky_top` / `sky_bottom`; the others are upload-once-and-
        // ignore until PR-3 prunes the struct. `bvh_n` reflects the
        // pool-wide primitive count so the shader's "scene-empty"
        // marker stays accurate when only streaming chunks are live.
        let meta = SceneMeta {
            primitive_count,
            bvh_n: self.bvh_state.total_primitive_count(),
            skip_internal_sky: u32::from(skip_internal_sky),
            has_intersects: u32::from(has_intersects),
            has_subs: u32::from(has_subs),
            k_int_scene: k_int_max,
            k_sub_scene: k_sub_max,
            _pad0: 0,
            sky_top: sky_top.to_array(),
            sky_bottom: sky_bottom.to_array(),
        };
        queue.write_buffer(&self.scene_meta_buffer, 0, bytemuck::bytes_of(&meta));
    }
}

