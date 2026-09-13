//! Summing the fields and handing the result to the solver.

mod apply;
mod collect;

use glam::Vec3;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::run_state::run_if_playing;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use crate::sources::{
    AreaGravity, BoxGravity, GlobalGravity, GravityPriority, PlaneGravity, PointGravity,
};

pub use apply::{apply_gravity_sources, reconcile_world_gravity_for_test};

/// The acceleration every source together applies at a world point — the same answer the solver
/// gets, for anything asking which way is down.
pub fn gravity_at(resources: &Resources, point: Vec3) -> Vec3 {
    collect::collect_sources(resources).acceleration_at(point)
}

/// Which way is up at a world point: away from the pull there, shared so every consumer agrees on
/// what near-zero means. World up where no field reaches, instead of a `NaN`.
pub fn gravity_up(resources: &Resources, point: Vec3) -> Vec3 {
    up_from(gravity_at(resources, point))
}

/// Which way is up according to the strongest source alone, after [`GravityPriority`] suppression.
/// For orientation only, never a force — [`gravity_up`] is the sum the solver applies.
pub fn gravity_dominant(resources: &Resources, point: Vec3) -> Vec3 {
    up_from(collect::collect_sources(resources).dominant_at(point))
}

/// World up where the pull is too small to have a direction — see
/// [`gravity_up`] for why that is not a `NaN`.
fn up_from(pull: Vec3) -> Vec3 {
    match pull.length_squared() < 1e-12 {
        true => Vec3::Y,
        false => -pull.normalize(),
    }
}

/// The components without the systems, for the editor: it mirrors and draws gravity but must never
/// apply it.
pub struct GravityComponentsPlugin;

impl Plugin for GravityComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<GlobalGravity>();
                registry.register_cpu_reflected::<PointGravity>();
                registry.register_cpu_reflected::<AreaGravity>();
                registry.register_cpu_reflected::<BoxGravity>();
                registry.register_cpu_reflected::<PlaneGravity>();
                registry.register_cpu_reflected::<GravityPriority>();
            }
        });
    }

    fn name(&self) -> &str {
        "GravityComponentsPlugin"
    }
}

/// Registers the gravity components and the system that applies them.
pub struct GravityPlugin;

impl Plugin for GravityPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(GravityComponentsPlugin);
        // Beside the solver, before it steps, so the impulse is for this step. Ungated, so a source
        // added while stopped applies from the first step of Play.
        app.add_system(Stage::PreUpdate, apply::reconcile_world_gravity);
        app.add_system(Stage::Physics, run_if_playing(apply_gravity_sources));
    }

    fn name(&self) -> &str {
        "GravityPlugin"
    }
}
