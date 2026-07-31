//! `RemoteSession` against a real in-process server — the connect
//! handshake and snapshot pull, without launching a child process.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::transform::Transform;

use kooch_editor_core::remote_session::{ConnectionState, RemoteSession};
use kooch_remote::handlers::handle;
use kooch_remote::protocol::Method;
use kooch_remote::server::RemoteServer;

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
        &kooch_remote::protocol::Request {
            id: 0,
            method: Method::Spawn {
                name: Some("Hero".into()),
            },
        },
        &mut resources,
    );
    resources
}

/// A socket name unique to this test.
///
/// Tests run in parallel in one process, so a shared name would have them
/// binding over each other — the local-socket equivalent of the port
/// scan this replaced, but solved instead of retried.
fn test_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    // The counter alone is not enough: it is per-module, so two test
    // modules in one binary both start at zero and collide on the same
    // name. The clock disambiguates without the modules having to know
    // about each other.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "kooch_test_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

#[test]
fn attach_connects_and_pulls_a_snapshot() {
    let server = RemoteServer::start(&test_socket_name()).expect("bind a port");
    let socket = server.name().to_owned();

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

    let mut session = RemoteSession::attach(&socket);
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
    // Attached to a socket nothing is listening on, a session never
    // leaves `Connecting`: there is no child process to mark it `Failed`,
    // and no server to answer the ping.
    let mut session = RemoteSession::attach("kooch_nothing_listens_here.sock");
    assert_eq!(session.poll_ready(), ConnectionState::Connecting);
}
