//! End-to-end tests: `handle` against a live ECS, and a full HTTP
//! round-trip through the listener thread and main-loop bridge.

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use kooch_remote::client::RemoteClient;
use kooch_remote::handlers::handle;
use kooch_remote::protocol::{Method, Request, ResponseData, ResponsePayload};
use kooch_remote::server::RemoteServer;

/// A minimal ECS with the resources the handlers touch, plus `Name` and
/// `Transform` registered as reflected components.
fn ecs() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(DynamicComponents::new());
    resources.insert(ComponentNames::new());
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<Transform>();
    // The hierarchy and ordering types `EcsPlugin` provides; without them `reparent` and `place`
    // silently do nothing.
    registry.register_cpu_reflected::<kooch_ecs::hierarchy::Parent>();
    registry.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
    registry.register_cpu_reflected::<kooch_ecs::Order>();
    resources
}

fn call(resources: &mut Resources, method: Method) -> ResponseData {
    let response = handle(
        &Request {
            id: 1,
            notify: false,
            method,
        },
        resources,
    );
    match response.payload {
        ResponsePayload::Result(data) => data,
        ResponsePayload::Error(e) => panic!("unexpected error: {e:?}"),
    }
}

/// A socket name unique to this test, since parallel tests would otherwise bind over each other.
fn test_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    // The counter is per module, so the clock disambiguates names across test modules in one
    // binary.
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

#[path = "integration/requests.rs"]
mod requests;
#[path = "integration/wire.rs"]
mod wire;
#[path = "integration/scenes.rs"]
mod scenes;
#[path = "integration/structure.rs"]
mod structure;
