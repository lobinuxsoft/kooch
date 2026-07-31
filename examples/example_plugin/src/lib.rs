//! A plugin, in the smallest form that still shows every moving part.
//!
//! Build it as a library the engine can load:
//!
//! ```text
//! RUSTFLAGS="-C prefer-dynamic" cargo build -p example_plugin
//! ```
//!
//! `prefer-dynamic` matters: without it the plugin links its own copy of
//! `std` and of the engine, and the two halves stop sharing globals —
//! including the log subscriber, so the plugin's output would vanish.

use kooch_plugin_api::prelude::*;

/// Counts frames, and keeps the count where a reload cannot lose it.
#[derive(Default)]
struct HelloPlugin;

/// Key under which the host holds our frame counter.
///
/// The count lives in the host on purpose. Anything a plugin keeps in
/// its own statics is unloaded with the library on the next reload;
/// state parked here survives it.
const FRAMES_KEY: &str = "example_plugin::frames";

impl OmePlugin for HelloPlugin {
    fn name(&self) -> &str {
        "HelloPlugin"
    }

    fn build(&mut self, engine: &mut dyn Engine) {
        engine.log("HelloPlugin loaded");

        // Declare a component type the engine has no Rust type for.
        // The editor draws it from this description alone.
        match engine.register_component(
            ComponentSchema::new("example_plugin::Health")
                .with_field("current", FieldKind::U32)
                .with_field("max", FieldKind::U32)
                .with_field("regen", FieldKind::F32),
        ) {
            Ok(()) => engine.log("registered example_plugin::Health"),
            Err(e) => engine.log(&format!("could not register Health: {e}")),
        }

        // A marker component: no fields is a legitimate shape.
        let _ = engine.register_component(ComponentSchema::new("example_plugin::Player"));

        engine.add_system(
            Stage::Update,
            Box::new(|engine: &mut dyn Engine| {
                let frames = engine
                    .get_data(FRAMES_KEY)
                    .and_then(|bytes| bytes.try_into().ok())
                    .map_or(0u64, u64::from_le_bytes);
                let frames = frames.wrapping_add(1);
                engine.set_data(FRAMES_KEY, &frames.to_le_bytes());

                // Every 60th frame, so the log stays readable.
                if frames % 60 == 0 {
                    engine.log(&format!("HelloPlugin has seen {frames} frames"));
                }
            }),
        );
    }

    fn cleanup(&mut self) {
        // Nothing to release: this plugin owns no state, by design.
    }
}

kooch_plugin_api::export_plugin!(HelloPlugin);
