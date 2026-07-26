//! [`PhysicsPlugin`] — wires the backend into the ECS and the schedule.

mod compound;
mod joints;
mod systems;
#[cfg(test)]
mod tests;
mod world;

pub use joints::JointRegistry;
pub use systems::{physics_step_system, physics_sync_system, physics_writeback_system};
pub use world::{BodySpec, PhysicsBody, PhysicsWorld};

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::run_state::run_if_playing;
use ome_core::stage::Stage;
use ome_ecs::component::ComponentRegistry;

use crate::components::{Collider, Joint, RigidBody};
use crate::rapier_backend::RapierBackend;

/// Adds rigid body physics to an app.
///
/// Inserts a [`PhysicsWorld`] wrapping a [`RapierBackend`], registers the
/// authored components so they show up in the Inspector, and schedules
/// the three systems described in [`systems`].
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

    /// Sets the world's unit of length, in metres.
    ///
    /// Leave it at 1 for a metre-scale world. A planet-scale world that
    /// authors in kilometres must say so, or every solver tolerance is
    /// three orders of magnitude too tight and stacks jitter for reasons
    /// that look nothing like a units problem.
    pub fn with_length_unit(mut self, metres: f32) -> Self {
        self.length_unit = metres;
        self
    }

    /// Sets the number of solver iterations per step.
    pub fn with_solver_iterations(mut self, iterations: usize) -> Self {
        self.solver_iterations = iterations;
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
/// [`PhysicsBody`] goes in unreflected on purpose — see its docs.
fn register_components(resources: &mut ome_core::resource::Resources) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<RigidBody>();
        registry.register_cpu_reflected::<Collider>();
        registry.register_cpu_reflected::<Joint>();
        registry.register_cpu::<PhysicsBody>();
    }
}

/// The physics components, with no simulation behind them.
///
/// For a host that has to *author* physics without running it: the editor
/// needs [`RigidBody`] and [`Collider`] reflected so they appear in the
/// add-component menu and the Inspector, but it must not stand up a
/// second solver. In remote mode its ECS is a mirror of a project that
/// owns the real physics world, and a local Rapier world full of mirrored
/// entities would be a second source of truth that simulates nothing.
///
/// Apps that want the simulation add [`PhysicsPlugin`], which includes
/// this.
pub struct PhysicsComponentsPlugin;

impl Plugin for PhysicsComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, register_components);
    }

    fn name(&self) -> &str {
        "PhysicsComponentsPlugin"
    }
}

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(PhysicsComponentsPlugin);
        app.insert_resource(PhysicsWorld::new(Box::new(self.backend())));

        // Sync runs unconditionally: the body set mirrors the ECS while
        // authoring too. Stepping and writeback are gameplay.
        app.add_system(Stage::PreUpdate, physics_sync_system);
        app.add_system(Stage::Physics, run_if_playing(physics_step_system));
        app.add_system(Stage::PostPhysics, run_if_playing(physics_writeback_system));
    }

    fn name(&self) -> &str {
        "PhysicsPlugin"
    }
}
