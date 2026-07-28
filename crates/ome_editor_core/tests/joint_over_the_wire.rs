//! Adding a `Joint` through the remote protocol.
//!
//! Reported symptom: after adding a Joint to an entity, the component
//! does not appear on it, the world stops updating, and the session has
//! to be reconnected. A joint with no bodies is *supposed* to warn and do
//! nothing — that part is by design. Nothing was supposed to stop.
//!
//! These tests take the same route the editor does: a real server, a real
//! `AddComponent`, and a real `list_entities` afterwards.

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::{ComponentNames, ComponentRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::name::Name;
use ome_ecs::query::AccessTracker;
use ome_ecs::transform::Transform;
use ome_physics::components::Joint;
use ome_remote::RemoteClient;
use ome_remote::handlers::handle;
use ome_remote::server::RemoteServer;

/// A socket name unique to this test.
fn test_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "ome_joint_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// An ECS with the components a project registers, `Joint` among them.
fn ecs() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<Joint>();
    r
}

/// The whole reported flow: spawn, add a Joint, list again.
///
/// **Currently failing on purpose** — it reproduces the reported bug and
/// is the check that the fix will have to satisfy. See the issue linked
/// in this file's module docs.
#[test]
#[ignore = "reproduces a known bug: EntityRef::Live cannot cross the protocol"]
fn a_joint_added_over_the_wire_comes_back_in_the_snapshot() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();

    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);
    let main_loop = std::thread::spawn(move || {
        let mut resources = ecs();
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let response = handle(&item.request, &mut resources);
                let _ = item.reply.send(response);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let client = RemoteClient::new(&socket);
    let entity = client.spawn(Some("Anchor")).expect("spawn");

    let joint_type = std::any::type_name::<Joint>();
    client
        .add_component(entity, joint_type)
        .expect("adding a Joint must not fail");

    // The snapshot after the add is the thing that was reported missing.
    let entities = client
        .list_entities()
        .expect("the snapshot must still be readable after adding a Joint");

    let listed = entities
        .iter()
        .find(|e| e.id == entity)
        .expect("the entity is still there");
    let joint = listed
        .components
        .iter()
        .find(|c| c.type_name.ends_with("Joint"));

    assert!(
        joint.is_some(),
        "Joint did not come back on the entity; components were: {:?}",
        listed
            .components
            .iter()
            .map(|c| c.type_name.as_str())
            .collect::<Vec<_>>()
    );

    // And the session keeps working afterwards, which is what "the
    // systems stopped" would deny.
    let again = client
        .list_entities()
        .expect("a second snapshot must still work");
    assert_eq!(again.len(), entities.len());

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// A Joint carries `EntityRef` fields, which are the ones most likely to
/// trip serialisation. Asserted separately so a failure names the cause.
#[test]
#[ignore = "reproduces a known bug: EntityRef::Live cannot cross the protocol"]
fn a_joints_fields_survive_the_round_trip() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let server = RemoteServer::start(&test_socket_name()).expect("bind");
    let socket = server.name().to_owned();

    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);
    let main_loop = std::thread::spawn(move || {
        let mut resources = ecs();
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let response = handle(&item.request, &mut resources);
                let _ = item.reply.send(response);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let client = RemoteClient::new(&socket);
    let entity = client.spawn(Some("Anchor")).expect("spawn");
    client
        .add_component(entity, std::any::type_name::<Joint>())
        .expect("add");

    let entities = client.list_entities().expect("list");
    let joint = entities
        .iter()
        .find(|e| e.id == entity)
        .and_then(|e| e.components.iter().find(|c| c.type_name.ends_with("Joint")))
        .expect("Joint present");

    let names: Vec<&str> = joint.fields.iter().map(|(n, _)| n.as_str()).collect();
    assert!(
        names.contains(&"body_a") && names.contains(&"body_b"),
        "the two entity references must survive; got {names:?}"
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

/// What exactly fails when a `Joint` is serialised?
///
/// The server falls back to writing `{}` when `to_string` errors, so the
/// cause never reaches anyone. This asks it directly, and names the field.
#[test]
#[ignore = "reproduces a known bug: EntityRef::Live cannot cross the protocol"]
fn a_joint_snapshot_serialises() {
    use ome_ecs::reflect::Reflect;

    let joint = Joint::default();
    let fields = joint.reflect_fields();

    for meta in fields {
        let value = joint.reflect_get(meta.name).expect("field readable");
        match ome_remote::serde_json::to_string(&value) {
            Ok(json) => {
                assert!(
                    !json.contains("null"),
                    "field `{}` serialised to null — a non-finite float does that, \
                     and it decodes back as a type error: {json}",
                    meta.name
                );
            }
            Err(e) => panic!("field `{}` failed to serialise: {e}", meta.name),
        }
    }
}
