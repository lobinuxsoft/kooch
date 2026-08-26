use super::*;
use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::build_default_meshlets;

fn cube_mesh() -> Mesh {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let face_indices: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [3, 2, 6, 7],
        [0, 3, 7, 4],
        [1, 2, 6, 5],
    ];
    let face_normal = [0.0, 1.0, 0.0];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for corners in face_indices {
        let base = vertices.len() as u32;
        for &c in &corners {
            vertices.push(MeshVertex {
                position: positions[c],
                normal: face_normal,
                uv: [0.0, 0.0],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::from_arrays(vertices, indices)
}

#[test]
fn register_is_idempotent() {
    let mut pipeline = MeshletPipeline::new();
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let guid = Guid::new_v4();

    let h0 = pipeline.register_mesh(guid, &mesh);
    let h1 = pipeline.register_mesh(guid, &mesh);
    assert_eq!(h0, h1);
    assert_eq!(pipeline.registered_count(), 1);
}

#[test]
fn distinct_guids_get_distinct_pool_handles() {
    let mut pipeline = MeshletPipeline::new();
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");

    let g1 = Guid::new_v4();
    let g2 = Guid::new_v4();
    let h1 = pipeline.register_mesh(g1, &mesh);
    let h2 = pipeline.register_mesh(g2, &mesh);
    assert_ne!(h1, h2);
    assert_eq!(pipeline.registered_count(), 2);
}

#[test]
fn lookup_returns_none_before_register() {
    let pipeline = MeshletPipeline::new();
    assert!(pipeline.lookup(Guid::new_v4()).is_none());
}

#[test]
fn instance_at_origin_uses_identity() {
    let inst = instance_at_origin(7);
    assert_eq!(inst.mesh_id, 7);
    let m = inst.transform_mat4();
    assert_eq!(m, Mat4::IDENTITY);
}

/// #492 regression: `MeshRenderer.visible == false` must be
/// filtered at the scene-collection step so the cull dispatch
/// never sees the entity. Same rule applies to
/// `collect_referenced_guids` — an invisible mesh should not be
/// pulled into the GPU pool either.
#[test]
fn invisible_mesh_renderer_is_filtered_at_collect() {
    use kooch_ecs::allocator::EntityAllocator;
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::commands::Commands;
    use kooch_ecs::component::registry::ComponentRegistry;
    use kooch_ecs::query::AccessTracker;

    let mut pipeline = MeshletPipeline::new();
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let guid = Guid::new_v4();
    pipeline.register_mesh(guid, &mesh);

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());

    let mut commands = Commands::new();
    // A — visible, valid mesh → should land in the instance vec.
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(glam::Vec3::ZERO),
        });
    // B — invisible: must be dropped at sync time.
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid),
            visible: false,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(glam::Vec3::new(2.0, 0.0, 0.0)),
        });
    // C — control: visible but mesh = None, must also be dropped
    // (separate filter inside collect_scene_instances).
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: None,
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(glam::Vec3::new(-2.0, 0.0, 0.0)),
        });
    commands.apply(&mut resources);

    let instances = pipeline.collect_scene_instances(&resources);
    assert_eq!(
        instances.len(),
        1,
        "only entity A (visible + valid mesh) should reach the instance vec"
    );

    let referenced = pipeline.collect_referenced_guids(&resources);
    assert_eq!(
        referenced.len(),
        1,
        "the invisible entity must not pull its mesh into the GPU pool"
    );
    assert_eq!(referenced[0], guid);
}
