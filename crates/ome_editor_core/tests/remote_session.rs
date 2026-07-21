//! `RemoteSession` against a real in-process server — the connect
//! handshake and snapshot pull, without launching a child process.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::{ComponentNames, ComponentRegistry};
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::name::Name;
use ome_ecs::query::AccessTracker;
use ome_ecs::transform::Transform;

use ome_editor_core::remote_session::{ConnectionState, RemoteSession};
use ome_remote::handlers::handle;
use ome_remote::protocol::Method;
use ome_remote::server::RemoteServer;

/// A minimal ECS with one named entity, mirroring a booted project.
fn seeded_ecs() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(DynamicComponents::new());
    resources.insert(ComponentNames::new());
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Name>();
        registry.register_cpu_reflected::<Transform>();
    }
    // Seed one entity so the snapshot is non-trivial.
    handle(
        &ome_remote::protocol::Request {
            id: 0,
            method: Method::Spawn {
                name: Some("Hero".into()),
            },
        },
        &mut resources,
    );
    resources
}

#[test]
fn attach_connects_and_pulls_a_snapshot() {
    let server = (0..8)
        .find_map(|i| RemoteServer::start(17740 + i).ok())
        .expect("bind a port");
    let port = server.port();

    let done = Arc::new(AtomicBool::new(false));
    let loop_done = Arc::clone(&done);
    let main_loop = std::thread::spawn(move || {
        let mut resources = seeded_ecs();
        while !loop_done.load(Ordering::Relaxed) {
            for item in server.take_pending() {
                let response = handle(&item.request, &mut resources);
                let _ = item.reply.send(response);
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    let mut session = RemoteSession::attach(port);
    assert_eq!(session.state(), ConnectionState::Connecting);

    // Drive the handshake; the server is up, so this connects promptly.
    let mut connected = false;
    for _ in 0..200 {
        if session.poll_ready() == ConnectionState::Connected {
            connected = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(connected, "session never connected");

    // The initial pull captured the seeded entity and the schema.
    assert_eq!(session.snapshot().len(), 1);
    assert_eq!(session.snapshot()[0].name.as_deref(), Some("Hero"));
    assert!(
        session
            .schema()
            .iter()
            .any(|c| c.type_name.ends_with("Transform")),
        "schema should list Transform"
    );

    // An edit issued through the session's client is visible on refresh.
    let hero = session.snapshot()[0].id;
    session
        .client()
        .add_component(hero, std::any::type_name::<Transform>())
        .expect("add component");
    session.refresh();
    assert!(
        session.snapshot()[0]
            .components
            .iter()
            .any(|c| c.type_name.ends_with("Transform")),
        "refresh should show the added component"
    );

    done.store(true, Ordering::Relaxed);
    main_loop.join().unwrap();
}

#[test]
fn attach_to_dead_port_stays_connecting() {
    // Attached to a port with no server, a session never leaves
    // `Connecting`: there is no child process to mark it `Failed`, and
    // no server to answer the ping.
    let mut session = RemoteSession::attach(59999);
    assert_eq!(session.poll_ready(), ConnectionState::Connecting);
}
