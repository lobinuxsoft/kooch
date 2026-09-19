/// A socket name unique to this test.
fn test_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static N: AtomicU32 = AtomicU32::new(0);
    // The counter alone is not enough: it is per-module, so two test modules in one binary both
    // start at zero and collide on the same name. The clock disambiguates without the modules
    // having to know about each other.
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

use std::any::TypeId;
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
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use kooch_remote::RemoteClient;
use kooch_remote::handlers::handle;
use kooch_remote::protocol::{Method, Request};
use kooch_remote::server::RemoteServer;

use super::*;
use crate::remote_session::{ConnectionState, RemoteSession, RemoteState};

fn ecs() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.insert(ComponentNames::new());
    {
        let reg = r.get_mut::<ComponentRegistry>().unwrap();
        reg.register_cpu_reflected::<Name>();
        reg.register_cpu_reflected::<Transform>();
    }
    r
}

/// The engine root, two levels above this crate's manifest.
fn engine_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kooch_editor_core is not two levels below the engine root")
        .to_path_buf()
}

/// An editor world that can resolve mesh assets out of `assets/`.
fn editor_with_assets() -> Resources {
    use kooch_core::asset_database::AssetDatabase;
    use kooch_core::asset_loader::AssetServer;
    use kooch_ecs::mesh_renderer::MeshRenderer;

    let mut r = ecs();
    {
        let reg = r.get_mut::<ComponentRegistry>().unwrap();
        reg.register_cpu_reflected::<MeshRenderer>();
    }
    let mut server = AssetServer::new().with_asset_root(engine_root().join("assets"));
    server.register_loader::<kooch_render::meshlet::MeshletMesh, _>(
        kooch_render::meshlet::MeshletMeshLoader,
    );
    r.insert(server);
    r.insert(AssetDatabase::new());
    r.insert(kooch_core::assets::Assets::<
        kooch_render::meshlet::MeshletMesh,
    >::new());
    r
}

mod routing;
mod spawning;
mod undo;
