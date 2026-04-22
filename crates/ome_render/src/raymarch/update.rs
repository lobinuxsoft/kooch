//! ECS → GPU upload: camera and SDF scene data.
//!
//! Collects all visible SDF shape entities into a unified instance
//! buffer, sorted by entity index for stable CSG evaluation order.
//! Uses `GlobalTransform` (world-space) so parent/child hierarchies
//! are respected — moving a parent moves its children with it.

use glam::{Mat4, Vec4};
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::{
    PerspectiveCamera, SdfBlend, SdfBox, SdfCapsule, SdfCylinder, SdfPlane, SdfSphere, SdfTorus,
};

use super::instance::{
    INITIAL_INSTANCE_CAPACITY, SceneMeta, SdfInstance, TYPE_BOX, TYPE_CAPSULE, TYPE_CYLINDER,
    TYPE_PLANE, TYPE_SPHERE, TYPE_TORUS,
};
use super::renderer::{RayMarchRenderer, make_scene_bg};

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

    /// Uploads every visible SDF shape entity to the instance storage buffer.
    /// Instances are sorted by `Entity::index()` to make CSG evaluation
    /// order reproducible across frames.
    pub fn update_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &ome_core::resource::Resources,
        sky_top: Vec4,
        sky_bottom: Vec4,
    ) {
        let mut tagged: Vec<(u32, SdfInstance)> = Vec::new();
        collect_spheres(resources, &mut tagged);
        collect_boxes(resources, &mut tagged);
        collect_capsules(resources, &mut tagged);
        collect_cylinders(resources, &mut tagged);
        collect_toruses(resources, &mut tagged);
        collect_planes(resources, &mut tagged);

        tagged.sort_by_key(|(idx, _)| *idx);
        let data: Vec<SdfInstance> = tagged.into_iter().map(|(_, i)| i).collect();

        let needed = data.len().max(1) as u64;
        if needed > self.instance_capacity {
            let new_cap = needed.next_power_of_two().max(INITIAL_INSTANCE_CAPACITY);
            self.instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("raymarch_instance_buffer"),
                size: new_cap * std::mem::size_of::<SdfInstance>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_capacity = new_cap;
            self.scene_bind_group = make_scene_bg(
                device,
                &self.scene_bind_group_layout,
                &self.scene_meta_buffer,
                &self.instance_buffer,
            );
        }

        if !data.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&data));
        }

        let meta = SceneMeta {
            instance_count: data.len() as u32,
            _pad0: [0; 3],
            sky_top: sky_top.to_array(),
            sky_bottom: sky_bottom.to_array(),
        };
        queue.write_buffer(&self.scene_meta_buffer, 0, bytemuck::bytes_of(&meta));
    }
}

fn blend_of(b: Option<&SdfBlend>) -> (u32, f32) {
    match b {
        Some(b) => (b.mode, b.smoothness),
        None => (0, 0.0),
    }
}

fn make_instance(
    entity: Entity,
    gt: &GlobalTransform,
    type_tag: u32,
    params: [f32; 4],
    blend: (u32, f32),
) -> (u32, SdfInstance) {
    let (scale, rotation, translation) = gt.matrix.to_scale_rotation_translation();
    let inst = SdfInstance {
        position: translation.to_array(),
        type_tag,
        rotation: rotation.to_array(),
        scale: scale.to_array(),
        _pad0: 0.0,
        params,
        blend_mode: blend.0,
        blend_smoothness: blend.1,
        _pad1: [0; 2],
    };
    (entity.index(), inst)
}

fn collect_spheres(
    resources: &ome_core::resource::Resources,
    out: &mut Vec<(u32, SdfInstance)>,
) {
    let q = Query::<(Entity, &SdfSphere, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, s, gt, b)| {
        if !s.visible {
            return;
        }
        out.push(make_instance(
            e,
            gt,
            TYPE_SPHERE,
            [s.radius, 0.0, 0.0, 0.0],
            blend_of(b),
        ));
    });
}

fn collect_boxes(
    resources: &ome_core::resource::Resources,
    out: &mut Vec<(u32, SdfInstance)>,
) {
    let q = Query::<(Entity, &SdfBox, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, b, gt, bl)| {
        if !b.visible {
            return;
        }
        out.push(make_instance(
            e,
            gt,
            TYPE_BOX,
            [b.size.x, b.size.y, b.size.z, b.rounding],
            blend_of(bl),
        ));
    });
}

fn collect_capsules(
    resources: &ome_core::resource::Resources,
    out: &mut Vec<(u32, SdfInstance)>,
) {
    let q = Query::<(Entity, &SdfCapsule, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, c, gt, bl)| {
        if !c.visible {
            return;
        }
        out.push(make_instance(
            e,
            gt,
            TYPE_CAPSULE,
            [c.half_height, c.radius, 0.0, 0.0],
            blend_of(bl),
        ));
    });
}

fn collect_cylinders(
    resources: &ome_core::resource::Resources,
    out: &mut Vec<(u32, SdfInstance)>,
) {
    let q = Query::<(Entity, &SdfCylinder, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, c, gt, bl)| {
        if !c.visible {
            return;
        }
        out.push(make_instance(
            e,
            gt,
            TYPE_CYLINDER,
            [c.half_height, c.radius, 0.0, 0.0],
            blend_of(bl),
        ));
    });
}

fn collect_toruses(
    resources: &ome_core::resource::Resources,
    out: &mut Vec<(u32, SdfInstance)>,
) {
    let q = Query::<(Entity, &SdfTorus, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, t, gt, bl)| {
        if !t.visible {
            return;
        }
        out.push(make_instance(
            e,
            gt,
            TYPE_TORUS,
            [t.major_radius, t.minor_radius, 0.0, 0.0],
            blend_of(bl),
        ));
    });
}

fn collect_planes(
    resources: &ome_core::resource::Resources,
    out: &mut Vec<(u32, SdfInstance)>,
) {
    let q = Query::<(Entity, &SdfPlane, &GlobalTransform, Option<&SdfBlend>)>::new(resources);
    q.for_each(|(e, p, gt, bl)| {
        if !p.visible {
            return;
        }
        out.push(make_instance(e, gt, TYPE_PLANE, [0.0; 4], blend_of(bl)));
    });
}
