//! Loads a built project's library the way the editor does, and reports
//! what it declared. Diagnoses the plugin path without a window.
//!
//! ```text
//! cargo run --example load_project_smoke --features editor -- <project-root> <crate-name>
//! ```
use oh_my_engine::ome_core::resource::Resources;
use oh_my_engine::ome_ecs::component::DynamicTypeRegistry;
use oh_my_engine::ome_editor_core::project_plugin::load_project_plugin;

fn main() {
    oh_my_engine::ome_core::init_tracing();

    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[1]);
    let crate_name = args.get(2).cloned().unwrap_or_else(|| "healthdemo".into());

    let mut resources = Resources::new();
    resources.insert(DynamicTypeRegistry::new());
    resources.insert(oh_my_engine::ome_core::dynamic::PluginData::new());
    resources.insert(oh_my_engine::ome_core::dynamic::ComponentBridge::new(
        |resources, schema| {
            let registry = resources
                .get_mut::<DynamicTypeRegistry>()
                .expect("registry");
            let source = schema
                .type_name
                .split("::")
                .next()
                .unwrap_or(&schema.type_name)
                .to_owned();
            oh_my_engine::ome_ecs::component::plugin_bridge::register_schema(
                registry, schema, &source,
            )
        },
    ));

    let gained = load_project_plugin(&mut resources, &root, &crate_name);
    println!("\n=== declared {gained} component type(s) ===");
    let registry = resources.get::<DynamicTypeRegistry>().unwrap();
    for ty in registry.iter() {
        println!("  {} (from {})", ty.type_name, ty.source);
        for field in &ty.fields {
            println!("      {}: {:?}", field.name, field.kind);
        }
    }
    if gained == 0 {
        eprintln!("NOTHING LOADED");
        std::process::exit(1);
    }
}
