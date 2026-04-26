//! ECS → GPU upload: camera and SDF scene data.
//!
//! Collects all visible SDF shape entities into a primitives storage
//! buffer (sorted by entity index for stable ordering), groups them by
//! CSG role (add / intersect / subtract) according to their optional
//! `SdfBlend`, builds a balanced default tree, and uploads its postfix
//! linearisation to a separate token storage buffer. The shader walks
//! the tokens once with a fixed-size evaluation stack — see
//! [`super::csg_tree`].

use glam::{Mat4, Vec4};
use ome_core::coord::ActiveOrigin;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::sdf_blend::{
    MODE_SMOOTH_INTERSECTION, MODE_SMOOTH_SUBTRACTION,
};
use ome_ecs::{
    PerspectiveCamera, SdfBlend, SdfBox, SdfCapsule, SdfCylinder, SdfPlane, SdfSphere, SdfTorus,
};

use super::csg_tree::{DefaultTreeRoles, Token, build_default_tree};
use super::instance::{
    INITIAL_PRIMITIVE_CAPACITY, INITIAL_TOKEN_CAPACITY, SceneMeta, SdfPrimitive, TYPE_BOX,
    TYPE_CAPSULE, TYPE_CYLINDER, TYPE_PLANE, TYPE_SPHERE, TYPE_TORUS,
};
use super::renderer::{RayMarchRenderer, make_scene_bg};

/// Per-entity blend metadata captured during ECS collection. Lives only
/// long enough to be folded into the default tree before upload.
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

    /// Uploads every visible SDF primitive entity + the postfix CSG
    /// token stream that composes them into a scene.
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
        // same primitive array + the same default tree (and therefore
        // the same token stream + byte-identical render output).
        tagged.sort_by_key(|(idx, _, _)| *idx);

        let mut primitives: Vec<SdfPrimitive> = Vec::with_capacity(tagged.len());
        let mut roles = DefaultTreeRoles::default();
        for (_, prim, blend) in tagged {
            let prim_idx = primitives.len() as u32;
            primitives.push(prim);
            match blend.mode {
                // ADD-like: MODE_REPLACE (hard union, k≈0) and
                // MODE_SMOOTH_UNION both go through the union pool.
                // Per-role k = max across the role's instances (issue
                // #307); MODE_REPLACE entities mixed with smooth ones
                // pick up the role's k — visually negligible drift on
                // legacy scenes, addressed when the tree-editor UX
                // ships and per-edge k becomes user-settable.
                MODE_SMOOTH_INTERSECTION => {
                    roles.intersects.push(prim_idx);
                    roles.intersect_smoothness_max =
                        roles.intersect_smoothness_max.max(blend.smoothness);
                }
                MODE_SMOOTH_SUBTRACTION => {
                    roles.subs.push(prim_idx);
                    roles.subtract_smoothness_max =
                        roles.subtract_smoothness_max.max(blend.smoothness);
                }
                _ => {
                    roles.adds.push(prim_idx);
                    roles.add_smoothness_max =
                        roles.add_smoothness_max.max(blend.smoothness);
                }
            }
        }

        // Build + serialise the default tree. Empty scenes (no adds, or
        // intersect/subtract without any add) collapse to zero tokens —
        // the shader treats this as a ray miss and renders the sky.
        let tokens: Vec<Token> = match build_default_tree(roles) {
            Some(tree) => match tree.serialise_postfix() {
                Ok(t) => t,
                Err(err) => {
                    tracing::warn!(
                        "raymarch: CSG tree serialisation failed ({err:?}); rendering empty scene"
                    );
                    Vec::new()
                }
            },
            None => Vec::new(),
        };

        // Resize buffers if needed, then re-bind. Both buffers grow
        // independently — adding a primitive doesn't necessarily change
        // the token count by the same amount.
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
        let token_needed = tokens.len().max(1) as u64;
        if token_needed > self.token_capacity {
            let new_cap = token_needed
                .next_power_of_two()
                .max(INITIAL_TOKEN_CAPACITY);
            self.tokens_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("raymarch_tokens_buffer"),
                size: new_cap * std::mem::size_of::<Token>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.token_capacity = new_cap;
            rebind = true;
        }
        if rebind {
            self.scene_bind_group = make_scene_bg(
                device,
                &self.scene_bind_group_layout,
                &self.scene_meta_buffer,
                &self.primitives_buffer,
                &self.tokens_buffer,
            );
        }

        if !primitives.is_empty() {
            queue.write_buffer(&self.primitives_buffer, 0, bytemuck::cast_slice(&primitives));
        }
        if !tokens.is_empty() {
            queue.write_buffer(&self.tokens_buffer, 0, bytemuck::cast_slice(&tokens));
        }

        let meta = SceneMeta {
            primitive_count: primitives.len() as u32,
            token_count: tokens.len() as u32,
            skip_internal_sky: u32::from(skip_internal_sky),
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
