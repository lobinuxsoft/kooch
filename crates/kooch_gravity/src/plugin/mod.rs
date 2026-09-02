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

/// The acceleration every source together applies at a world point.
///
/// Public because it is the honest way to ask "which way is down here" —
/// a character controller aligning to a planet needs the same answer the
/// solver gets, and recomputing it differently is how the two disagree.
pub fn gravity_at(resources: &Resources, point: Vec3) -> Vec3 {
    collect::collect_sources(resources).acceleration_at(point)
}

/// Which way is up at a world point: away from the pull acting there.
///
/// Every consumer of [`gravity_at`] needs this same three lines —
/// normalise, negate, and decide what "no field" means — so it lives
/// here rather than in each of them. It had been written twice already
/// (the camera's `up_mode = Gravity` and a game's movement plane) and the
/// two copies had drifted to different thresholds for "close enough to
/// zero", which is the whole failure mode of a duplicated decision.
///
/// # Where there is no field
///
/// Returns world up. `gravity_at` gives a zero vector where nothing
/// reaches, and normalising that is a `NaN` that spreads to a camera
/// pose or an impulse and shows up somewhere else entirely. Free space
/// has no better answer, and an arbitrary-but-stable one keeps controls
/// predictable instead of undefined.
pub fn gravity_up(resources: &Resources, point: Vec3) -> Vec3 {
    up_from(gravity_at(resources, point))
}

/// Which way is up according to the strongest single source, ignoring
/// every weaker one.
///
/// [`gravity_up`] answers with the sum, and that is the right default: it
/// is what the solver applies, so a body and a camera agree about down.
/// Between two planets of similar pull the sum points at neither, which is
/// physically correct and reads as a character standing at a slant in
/// empty space.
///
/// This one snaps to whichever source is winning. It is for orientation —
/// which way a character's feet point — and never for a force. Using it to
/// move something would apply a pull the solver is not applying.
///
/// Suppression from [`GravityPriority`] is applied first, so a source
/// overruled by a zone above it cannot be the dominant one.
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

/// The components without the systems, for a host that authors gravity
/// but does not simulate it.
///
/// The editor is that host: the solver lives in the project's process, so
/// this side needs the fields to exist as data — to mirror, inspect and
/// draw — and must never apply them.
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
        // In the fixed stage beside the solver, before it steps: the
        // impulse is for this step, and applying it after would move the
        // body a step late.
        // Ungated: a source added while stopped has to take effect before
        // the first step, or the first frame of Play uses the old world
        // vector.
        app.add_system(Stage::PreUpdate, apply::reconcile_world_gravity);
        app.add_system(Stage::Physics, run_if_playing(apply_gravity_sources));
    }

    fn name(&self) -> &str {
        "GravityPlugin"
    }
}
