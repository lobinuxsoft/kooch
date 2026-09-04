use super::*;

use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_physics::components::{SHAPE_CONVEX_HULL, SHAPE_SPHERE};

/// Resources with the component storage the walk reads, and an empty
/// cache. No asset server: every test here is about *which* GUIDs get
/// asked for, which is the half that needs no I/O.
fn world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(Commands::new());
    resources.insert(ColliderMeshCache::new());
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Collider>();
    resources
}

/// The GUIDs the walk asks for, without the per-shape detail.
fn guids(resources: &Resources) -> Vec<Guid> {
    unanswered(resources).into_iter().map(|w| w.guid).collect()
}

fn with_collider(resources: &mut Resources, collider: Collider) {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Collider>()
    {
        storage.insert(entity, collider);
    }
}

#[test]
fn a_sphere_asks_for_nothing() {
    let mut resources = world();
    with_collider(
        &mut resources,
        Collider {
            shape: SHAPE_SPHERE,
            mesh: Some(Guid::new_v4()),
            ..Default::default()
        },
    );
    assert!(
        unanswered(&resources).is_empty(),
        "a shape that reads no mesh must not load one",
    );
}

#[test]
fn a_hull_asks_for_its_mesh() {
    let mut resources = world();
    let guid = Guid::new_v4();
    with_collider(
        &mut resources,
        Collider {
            shape: SHAPE_CONVEX_HULL,
            mesh: Some(guid),
            ..Default::default()
        },
    );
    assert_eq!(guids(&resources), vec![guid]);
}

/// A hull is the one shape that pays for a reduction, and the reduction
/// costs 33 ms on a 76k mesh. A trimesh must not ask for one.
#[test]
fn only_a_hull_asks_for_the_reduction() {
    let mut resources = world();
    with_collider(
        &mut resources,
        Collider {
            shape: kooch_physics::components::SHAPE_TRIMESH,
            mesh: Some(Guid::new_v4()),
            ..Default::default()
        },
    );
    assert_eq!(unanswered(&resources).len(), 1);
    assert!(!unanswered(&resources)[0].hull);
}

/// Two colliders on one mesh, one of them a hull: the hull is wanted.
/// Taking the first answer seen would make it depend on hash order.
#[test]
fn one_hull_among_many_still_reduces() {
    let mut resources = world();
    let guid = Guid::new_v4();
    for shape in [
        kooch_physics::components::SHAPE_TRIMESH,
        SHAPE_CONVEX_HULL,
        kooch_physics::components::SHAPE_POLYLINE,
    ] {
        with_collider(
            &mut resources,
            Collider {
                shape,
                mesh: Some(guid),
                ..Default::default()
            },
        );
    }
    let wanted = unanswered(&resources);
    assert_eq!(wanted.len(), 1, "asked once for one mesh");
    assert!(wanted[0].hull);
}

/// A scene where a hundred crates share one collision mesh does one load,
/// and a failure is an answer too — otherwise a broken GUID is retried
/// every frame forever.
#[test]
fn an_answered_guid_is_not_asked_again() {
    let mut resources = world();
    let guid = Guid::new_v4();
    for _ in 0..3 {
        with_collider(
            &mut resources,
            Collider {
                shape: SHAPE_CONVEX_HULL,
                mesh: Some(guid),
                ..Default::default()
            },
        );
    }
    assert_eq!(guids(&resources), vec![guid], "asked once, not thrice");

    resources.get_mut::<ColliderMeshCache>().unwrap().fail(guid);
    assert!(unanswered(&resources).is_empty());
}

/// Nothing to resolve must cost nothing, and must not remove the cache
/// from `Resources` on the way past.
#[test]
fn an_idle_frame_keeps_the_cache() {
    let mut resources = world();
    fill_collider_meshes(&mut resources);
    assert!(resources.get::<ColliderMeshCache>().is_some());
}

/// 🔴 A block publishes its own collider; this walk must not try to read
/// its `.block` as glTF.
///
/// It did, the parse failed, and `fail` is permanent — `answered` counts
/// a failure as an answer — so the body never collided even after the
/// real entry landed a frame later.
#[test]
fn a_non_mesh_guid_is_not_asked_for() {
    use kooch_core::asset_database::{AssetDatabase, AssetEntry};

    let mut resources = world();
    let guid = Guid::new_v4();
    let mut database = AssetDatabase::new();
    database.register(
        guid,
        AssetEntry {
            path: std::path::PathBuf::from("assets/blocks/Wall.block"),
            mtime: std::time::SystemTime::UNIX_EPOCH,
            type_name: Some("kooch_blockmesh::block_mesh::BlockMesh".to_owned()),
        },
    );
    resources.insert(database);

    with_collider(
        &mut resources,
        Collider {
            shape: SHAPE_CONVEX_HULL,
            mesh: Some(guid),
            ..Default::default()
        },
    );

    assert!(guids(&resources).is_empty(), "a block is not glTF");
}

/// An untyped GUID is still asked for: the type lands on the entry the
/// first time something loads it, so refusing earlier would stop a mesh
/// from ever being read.
#[test]
fn an_untyped_guid_is_still_asked_for() {
    let mut resources = world();
    let guid = Guid::new_v4();
    with_collider(
        &mut resources,
        Collider {
            shape: SHAPE_CONVEX_HULL,
            mesh: Some(guid),
            ..Default::default()
        },
    );

    assert_eq!(guids(&resources), vec![guid]);
}
