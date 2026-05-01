//! ECS → GPU primitive collection. Walks every `Sdf*` component type
//! the engine knows about (sphere, box, capsule, cylinder, torus,
//! plane) paired with `GlobalTransform` + optional `SdfBlend`, and
//! materialises a flat list of `(entity_index, gpu_primitive, blend)`
//! tuples that `update_scene` sorts and folds into the BVH input.
//!
//! Split out of `update.rs` to keep both files under the 400-LoC
//! monolithic threshold — see #379. The split is along the natural
//! seam: collection is a pure ECS read, the upload + BVH-tick logic
//! that consumes the collected list lives in `update.rs`.
//!
//! Determinism: `update_scene` sorts the returned list by entity
//! index before serialising into the primitives buffer, so the GPU
//! sees a stable ordering regardless of ECS storage layout.
//!
//! Visibility: `pub(super)` everything — only the parent
//! `raymarch::update` module consumes these helpers.
use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::{
    SdfBlend, SdfBox, SdfCapsule, SdfCylinder, SdfPlane, SdfSphere, SdfTorus,
};

use super::instance::{
    SdfPrimitive, TYPE_BOX, TYPE_CAPSULE, TYPE_CYLINDER, TYPE_PLANE, TYPE_SPHERE, TYPE_TORUS,
};

/// Per-entity blend metadata captured during ECS collection. Lives
/// only long enough to be folded into the leaf-AABB table before
/// upload.
#[derive(Copy, Clone, Debug)]
pub(super) struct BlendInfo {
    pub(super) mode: u32,
    pub(super) smoothness: f32,
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
pub(super) type CollectedRow = (u32, SdfPrimitive, BlendInfo);

/// Walk every visible `Sdf*` component in `resources` and return the
/// flat collection list. Caller sorts by entity index for deterministic
/// upload ordering.
pub(super) fn collect_all_visible_sdfs(resources: &Resources) -> Vec<CollectedRow> {
    let mut tagged: Vec<CollectedRow> = Vec::new();
    collect_spheres(resources, &mut tagged);
    collect_boxes(resources, &mut tagged);
    collect_capsules(resources, &mut tagged);
    collect_cylinders(resources, &mut tagged);
    collect_toruses(resources, &mut tagged);
    collect_planes(resources, &mut tagged);
    tagged
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
        // Populated in the second pass — needs the per-role k_max
        // tally that's not available at primitive-collection time.
        smoothness: 0.0,
        params,
    };
    (entity.index(), prim, blend)
}

fn collect_spheres(resources: &Resources, out: &mut Vec<CollectedRow>) {
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

fn collect_boxes(resources: &Resources, out: &mut Vec<CollectedRow>) {
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

fn collect_capsules(resources: &Resources, out: &mut Vec<CollectedRow>) {
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

fn collect_cylinders(resources: &Resources, out: &mut Vec<CollectedRow>) {
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

fn collect_toruses(resources: &Resources, out: &mut Vec<CollectedRow>) {
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

fn collect_planes(resources: &Resources, out: &mut Vec<CollectedRow>) {
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
