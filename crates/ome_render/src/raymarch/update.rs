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

use glam::{Mat4, Vec4};
use ome_bvh::Aabb;
use ome_core::coord::ActiveOrigin;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::sdf_blend::{MODE_SMOOTH_INTERSECTION, MODE_SMOOTH_SUBTRACTION};
use ome_ecs::{
    PerspectiveCamera, SdfBlend, SdfBox, SdfCapsule, SdfCylinder, SdfPlane, SdfSphere, SdfTorus,
};

use super::aabb::primitive_aabb;
use super::instance::{
    IS_RAYMARCH, INITIAL_PRIMITIVE_CAPACITY, LeafAabb, RaymarchPayload, ROLE_RAYMARCH_ADD,
    ROLE_RAYMARCH_INT, ROLE_RAYMARCH_SUB, SceneMeta, SdfPrimitive, TYPE_BOX, TYPE_CAPSULE,
    TYPE_CYLINDER, TYPE_PLANE, TYPE_SPHERE, TYPE_TORUS,
};
use super::renderer::{RayMarchRenderer, make_scene_bg};

/// Per-entity blend metadata captured during ECS collection. Lives only
/// long enough to be folded into the leaf-AABB table before upload.
#[derive(Copy, Clone, Debug)]
struct BlendInfo {
    mode: u32,
    smoothness: f32,
}

impl BlendInfo {
    fn from_component(b: Option<&SdfBlend>) -> Self {
        match b {
            Some(b) => Self {
                mode: b.mode,
                smoothness: b.smoothness,
            },
            None => Self {
                mode: 0,
                smoothness: 0.0,
            },
        }
    }
}

/// One entry in the per-frame collection list: tag (entity index for
/// stable ordering) + the GPU primitive bytes + its blend metadata.
type CollectedRow = (u32, SdfPrimitive, BlendInfo);

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

        let uniforms = super::instance::CameraUniforms {
            view: view.to_cols_array_2d(),
            projection: projection.to_cols_array_2d(),
            inverse_view: view.inverse().to_cols_array_2d(),
            inverse_projection: projection.inverse().to_cols_array_2d(),
            position: translation.to_array(),
            _pad0: 0.0,
        };
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniforms));
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
        true
    }

    /// Uploads every visible SDF primitive entity, kicks a BVH rebuild
    /// when the scene changed, and refreshes the scene bind group.
    ///
    /// `skip_internal_sky = true` tells the fragment shader to discard on
    /// ray miss instead of drawing its internal gradient — use this when a
    /// separate sky pass ran before the raymarch and already filled the
    /// background.
    pub fn update_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &ome_core::resource::Resources,
        sky_top: Vec4,
        sky_bottom: Vec4,
        skip_internal_sky: bool,
    ) {
        let mut tagged: Vec<CollectedRow> = Vec::new();
        collect_spheres(resources, &mut tagged);
        collect_boxes(resources, &mut tagged);
        collect_capsules(resources, &mut tagged);
        collect_cylinders(resources, &mut tagged);
        collect_toruses(resources, &mut tagged);
        collect_planes(resources, &mut tagged);

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
        for (entity_idx, prim, blend) in tagged {
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
            primitives.push(prim);
        }

        // Resize the primitives buffer if needed. The leaf-AABB and BVH
        // node buffers live inside `bvh_state`'s slot management.
        let mut rebind = false;
        let prim_needed = primitives.len().max(1) as u64;
        if prim_needed > self.primitive_capacity {
            let new_cap = prim_needed
                .next_power_of_two()
                .max(INITIAL_PRIMITIVE_CAPACITY);
            self.primitives_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("raymarch_primitives_buffer"),
                size: new_cap * std::mem::size_of::<SdfPrimitive>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.primitive_capacity = new_cap;
            rebind = true;
        }

        if !primitives.is_empty() {
            queue.write_buffer(&self.primitives_buffer, 0, bytemuck::cast_slice(&primitives));
        }

        // Drive the BVH state: poll any in-flight build for completion
        // (potentially flipping `current_slot`), then kick a new build
        // if the scene state changed since the last successful kick.
        // Both calls are idempotent on stable scenes.
        if let Some(Err(e)) = self.bvh_state.poll_swap(device, queue) {
            tracing::warn!(
                "raymarch BVH build failed: {e}; keeping previous slot's data"
            );
        }
        self.bvh_state.kick_if_dirty(
            device,
            queue,
            bvh_items,
            leaf_aabbs,
            raymarch_payloads,
        );

        // Rebind every frame: `current_slot` may have flipped between
        // two GPU buffer sets (slot_a ↔ slot_b) and the bind group must
        // point at the slot the renderer is consuming this frame.
        // wgpu treats bind-group creation as cheap; this is the simple
        // path. Capacity-growth path also flows through here.
        let _ = rebind;
        self.scene_bind_group = make_scene_bg(
            device,
            &self.scene_bind_group_layout,
            &self.scene_meta_buffer,
            &self.primitives_buffer,
            self.bvh_state.current_nodes(),
            self.bvh_state.current_sorted_indices(),
            self.bvh_state.current_leaf_aabbs(),
            self.bvh_state.current_raymarch_payloads(),
        );

        let meta = SceneMeta {
            primitive_count: primitives.len() as u32,
            bvh_n: self.bvh_state.current_n(),
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

fn make_primitive(
    entity: Entity,
    gt: &GlobalTransform,
    type_tag: u32,
    params: [f32; 4],
    blend: BlendInfo,
) -> CollectedRow {
    let (scale, rotation, translation) = gt.matrix.to_scale_rotation_translation();
    let prim = SdfPrimitive {
        position: translation.to_array(),
        type_tag,
        rotation: rotation.to_array(),
        scale: scale.to_array(),
        _pad0: 0.0,
        params,
    };
    (entity.index(), prim, blend)
}

fn collect_spheres(resources: &ome_core::resource::Resources, out: &mut Vec<CollectedRow>) {
    let q = Query::<(Entity, &SdfSphere, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, s, gt, b)| {
        if !s.visible {
            return;
        }
        out.push(make_primitive(
            e,
            gt,
            TYPE_SPHERE,
            [s.radius, 0.0, 0.0, 0.0],
            BlendInfo::from_component(b),
        ));
    });
}

fn collect_boxes(resources: &ome_core::resource::Resources, out: &mut Vec<CollectedRow>) {
    let q = Query::<(Entity, &SdfBox, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, b, gt, bl)| {
        if !b.visible {
            return;
        }
        out.push(make_primitive(
            e,
            gt,
            TYPE_BOX,
            [b.size.x, b.size.y, b.size.z, b.rounding],
            BlendInfo::from_component(bl),
        ));
    });
}

fn collect_capsules(resources: &ome_core::resource::Resources, out: &mut Vec<CollectedRow>) {
    let q = Query::<(Entity, &SdfCapsule, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, c, gt, bl)| {
        if !c.visible {
            return;
        }
        out.push(make_primitive(
            e,
            gt,
            TYPE_CAPSULE,
            [c.half_height, c.radius, 0.0, 0.0],
            BlendInfo::from_component(bl),
        ));
    });
}

fn collect_cylinders(resources: &ome_core::resource::Resources, out: &mut Vec<CollectedRow>) {
    let q = Query::<(Entity, &SdfCylinder, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, c, gt, bl)| {
        if !c.visible {
            return;
        }
        out.push(make_primitive(
            e,
            gt,
            TYPE_CYLINDER,
            [c.half_height, c.radius, 0.0, 0.0],
            BlendInfo::from_component(bl),
        ));
    });
}

fn collect_toruses(resources: &ome_core::resource::Resources, out: &mut Vec<CollectedRow>) {
    let q = Query::<(Entity, &SdfTorus, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, t, gt, bl)| {
        if !t.visible {
            return;
        }
        out.push(make_primitive(
            e,
            gt,
            TYPE_TORUS,
            [t.major_radius, t.minor_radius, 0.0, 0.0],
            BlendInfo::from_component(bl),
        ));
    });
}

fn collect_planes(resources: &ome_core::resource::Resources, out: &mut Vec<CollectedRow>) {
    let q = Query::<(Entity, &SdfPlane, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, p, gt, bl)| {
        if !p.visible {
            return;
        }
        out.push(make_primitive(
            e,
            gt,
            TYPE_PLANE,
            [0.0; 4],
            BlendInfo::from_component(bl),
        ));
    });
}
