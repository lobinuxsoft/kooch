//! That a block's collider reaches the solver before the solver reads it (#1316).

use kooch_core::app::App;
use kooch_core::stage::Stage;

/// Where a name sits in `PrePhysics`, or `None` when nothing answers to it.
fn at(app: &App, name: &str) -> Option<usize> {
    app.schedule
        .system_names(Stage::PrePhysics)
        .iter()
        .position(|system| system.contains(name))
}

/// 🔴 The order must come from the constraint, not from which plugin was added first. Adding
/// physics FIRST is the arrangement that used to lose: both systems sat in `PreUpdate`, order
/// within a stage is registration order, and a block's mesh reached the solver a frame late
/// without anything failing.
#[test]
fn geometry_precedes_the_sync() {
    for physics_first in [true, false] {
        let mut app = App::new();
        app.add_plugin(kooch_ecs::plugin::EcsPlugin);
        match physics_first {
            true => {
                app.add_plugin(kooch_physics::plugin::PhysicsPlugin::new());
                app.add_plugin(crate::BlockPlugin);
            }
            false => {
                app.add_plugin(crate::BlockPlugin);
                app.add_plugin(kooch_physics::plugin::PhysicsPlugin::new());
            }
        }
        let blocks = at(&app, "sync_blocks").expect("sync_blocks is in PrePhysics");
        let sync = at(&app, "physics_sync_system").expect("physics_sync_system is in PrePhysics");
        assert!(
            blocks < sync,
            "physics added first: {physics_first} — a block's collider is built at {blocks}, \
             after the sync at {sync} that reads it",
        );
    }
}

/// And both run behind the propagation, since the sync reads the world pose it publishes.
#[test]
fn propagation_precedes_the_sync() {
    let mut app = App::new();
    app.add_plugin(kooch_ecs::plugin::EcsPlugin);
    app.add_plugin(kooch_physics::plugin::PhysicsPlugin::new());
    let propagation =
        at(&app, "transform_propagation_system").expect("propagation is in PrePhysics");
    let sync = at(&app, "physics_sync_system").expect("physics_sync_system is in PrePhysics");
    assert!(
        propagation < sync,
        "the sync at {sync} reads transforms propagated at {propagation}",
    );
}
