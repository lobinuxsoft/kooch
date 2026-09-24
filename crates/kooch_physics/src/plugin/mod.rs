//! [`PhysicsPlugin`] — wires the backend into the ECS and the schedule.

mod compound;
pub(super) mod events;
mod joints;
pub(super) mod sensors;
mod systems;
#[cfg(test)]
mod tests;
mod world;

pub use events::{CollisionStarted, CollisionStopped, ContactForce, JointBroke};
pub use joints::JointRegistry;
pub use systems::{physics_step_system, physics_sync_system, physics_writeback_system};
pub use world::{BodySpec, PhysicsWorld, SolverBody};

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::run_state::run_if_playing;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use crate::backend::ColliderMeshCache;
use crate::components::{Collider, Joint, PhysicsBody};
use crate::rapier_backend::RapierBackend;

/// Adds rigid body physics to an app.
///
/// Inserts a [`PhysicsWorld`] wrapping a [`RapierBackend`], registers the
/// authored components so they show up in the Inspector, and schedules
/// the three systems described in `systems`.
///
/// # Example
///
/// ```ignore
/// app.add_plugin(
///     PhysicsPlugin::new()
///         .with_gravity(Vec3::new(0.0, -3.71, 0.0)) // Mars
///         .with_length_unit(1000.0)                 // authoring in km
/// );
/// ```
pub struct PhysicsPlugin {
    gravity: glam::Vec3,
    length_unit: f32,
    solver_iterations: usize,
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsPlugin {
    /// Earth gravity, metre-scale, Rapier's default solver iterations.
    pub fn new() -> Self {
        let defaults = RapierBackend::new();
        Self {
            gravity: defaults.gravity(),
            length_unit: defaults.length_unit(),
            solver_iterations: defaults.solver_iterations(),
        }
    }

    /// Sets the world's gravity vector, in world units per second squared.
    pub fn with_gravity(mut self, gravity: glam::Vec3) -> Self {
        self.gravity = gravity;
        self
    }

    /// The world's unit of length in metres; a kilometre world at 1 gets tolerances 1000× too tight
    /// and jittering stacks.
    pub fn with_length_unit(mut self, metres: f32) -> Self {
        self.length_unit = metres;
        self
    }

    /// Builds the configured backend.
    fn backend(&self) -> RapierBackend {
        let mut backend = RapierBackend::new();
        backend.set_gravity(self.gravity);
        backend.set_length_unit(self.length_unit);
        backend.set_solver_iterations(self.solver_iterations);
        backend
    }
}

/// Registers the authored components and the runtime slot component.
///
/// [`SolverBody`] goes in unreflected on purpose — see its docs.
fn register_components(resources: &mut kooch_core::resource::Resources) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<PhysicsBody>();
        registry.register_cpu_reflected::<Collider>();
        registry.register_cpu_reflected::<Joint>();
        registry.register_cpu::<SolverBody>();
    }
}

/// Physics components reflected without a solver — for the editor, whose remote ECS mirrors a
/// project owning the real world. [`PhysicsPlugin`] includes it.
pub struct PhysicsComponentsPlugin;

impl Plugin for PhysicsComponentsPlugin {
    fn build(&self, app: &mut App) {
        // Empty, and inserted even where nothing fills it: a mesh-derived
        // collider resolves to no geometry rather than to a stand-in, so
        // an absent cache and an unfilled one have to behave the same.
        app.insert_resource(ColliderMeshCache::new());
        app.add_system(Stage::Startup, register_components);
        // Who is inside which region, measured rather than solved: this plugin is what a host
        // without a solver adds, and a post-process volume has to preview there too (#1222).
        app.insert_resource(kooch_ecs::sensor_occupancy::SensorOccupancy::default());
        app.add_system(Stage::PreUpdate, sensors::sensor_occupancy_preview_system);
    }

    fn name(&self) -> &str {
        "PhysicsComponentsPlugin"
    }
}

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(PhysicsComponentsPlugin);
        app.insert_resource(PhysicsWorld::new(Box::new(self.backend())));

        // Lifecycle before sync, ungated: a play-gated system cannot see play end. Sync runs always
        // (bodies mirror the ECS while authoring); step and writeback are gameplay.
        app.add_system(Stage::PreUpdate, events::physics_lifecycle_system);
        app.add_system(Stage::PreUpdate, physics_sync_system);
        app.add_system(Stage::Physics, run_if_playing(physics_step_system));
        app.add_system(Stage::PostPhysics, run_if_playing(physics_writeback_system));

        // The solver's reports, delivered after the step rather than during
        // it — see `events`. Registered here so a host that adds physics
        // gets the buffers without a second call to remember.
        app.add_event::<CollisionStarted>();
        app.add_event::<CollisionStopped>();
        app.add_event::<ContactForce>();
        app.add_event::<JointBroke>();
        app.add_system(
            Stage::PostPhysics,
            run_if_playing(events::drain_physics_events),
        );
        // After the drain, in the same stage and the same cadence: the arrivals it just published
        // are this step's, read from the fixed buffers (#1312).
        app.insert_resource(kooch_ecs::sensor_occupancy::SensorOccupancy::default());
        app.add_system(
            Stage::PostPhysics,
            run_if_playing(sensors::sensor_occupancy_system),
        );
    }

    fn name(&self) -> &str {
        "PhysicsPlugin"
    }
}
