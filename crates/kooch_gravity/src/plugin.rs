//! Summing the fields and handing the result to the solver.

use glam::Vec3;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::run_state::run_if_playing;
use kooch_core::stage::Stage;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::transform::Transform;

use crate::sources::{AreaGravity, BoxGravity, GlobalGravity, PointGravity};

/// One source, resolved into world space.
struct Source {
    position: Vec3,
    kind: Kind,
}

enum Kind {
    Global(GlobalGravity),
    Point(PointGravity),
    /// Carried with both halves of its transform: the inverse puts a
    /// world point into the box's space, and the rotation brings the
    /// resulting direction back out.
    ///
    /// Both, because only converting the point is a bug that looks like
    /// the field working — the box is tested in the right place and then
    /// pulls along the *unrotated* direction, so a rotated zone drops
    /// things straight down.
    Area {
        settings: AreaGravity,
        local: LocalSpace,
    },
    /// A solid whose direction differs at every point around it, so its
    /// answer needs the same round trip an area's does.
    Solid {
        settings: BoxGravity,
        local: LocalSpace,
    },
}

/// The two halves of a transform a local-space field needs: the inverse to
/// put a world point into the field's space, and the rotation to bring the
/// resulting direction back out.
///
/// Both, because only converting the point is a bug that looks like the
/// field working — the shape is tested in the right place and then pulls
/// along an *unrotated* direction, so a rotated zone drops things straight
/// down.
struct LocalSpace {
    to_local: glam::Mat4,
    rotation: glam::Quat,
}

impl LocalSpace {
    fn direction_from(&self, point: Vec3, local: impl Fn(Vec3) -> Vec3) -> Vec3 {
        self.rotation * local(self.to_local.transform_point3(point))
    }
}

/// The acceleration every source together applies at a world point.
///
/// Public because it is the honest way to ask "which way is down here" —
/// a character controller aligning to a planet needs the same answer the
/// solver gets, and recomputing it differently is how the two disagree.
pub fn gravity_at(resources: &Resources, point: Vec3) -> Vec3 {
    collect_sources(resources)
        .iter()
        .map(|source| source.acceleration_at(point))
        .sum()
}

impl Source {
    fn acceleration_at(&self, point: Vec3) -> Vec3 {
        match &self.kind {
            Kind::Global(global) => global.acceleration,
            Kind::Point(source) => source.acceleration_at(self.position, point),
            Kind::Area { settings, local } => {
                local.direction_from(point, |p| settings.acceleration_at_local(p))
            }
            Kind::Solid { settings, local } => {
                local.direction_from(point, |p| settings.acceleration_at_local(p))
            }
        }
    }
}

/// Reads every source in the world, in world space.
fn collect_sources(resources: &Resources) -> Vec<Source> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let globals = registry.get_cpu::<GlobalTransform>();
    let transforms = registry.get_cpu::<Transform>();

    // A source without a `GlobalTransform` falls back to its `Transform`,
    // and then to the origin: a global field does not care where it is, and
    // refusing to work until the hierarchy has propagated would make a
    // freshly spawned planet silently inert for a frame.
    let position_of = |entity: Entity| -> Vec3 {
        globals
            .and_then(|storage| storage.get(entity))
            .map(|global| global.matrix.to_scale_rotation_translation().2)
            .or_else(|| transforms.and_then(|s| s.get(entity)).map(|t| t.position))
            .unwrap_or(Vec3::ZERO)
    };
    let matrix_of = |entity: Entity| -> glam::Mat4 {
        globals
            .and_then(|storage| storage.get(entity))
            .map(|global| global.matrix)
            .or_else(|| {
                transforms.and_then(|s| s.get(entity)).map(|t| {
                    glam::Mat4::from_scale_rotation_translation(t.scale, t.rotation, t.position)
                })
            })
            .unwrap_or(glam::Mat4::IDENTITY)
    };

    let mut sources = Vec::new();
    if let Some(storage) = registry.get_cpu::<GlobalGravity>() {
        for (&entity, global) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                kind: Kind::Global(*global),
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<PointGravity>() {
        for (&entity, point) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                kind: Kind::Point(*point),
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<AreaGravity>() {
        for (&entity, area) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                kind: Kind::Area {
                    settings: *area,
                    local: local_space(matrix_of(entity)),
                },
            });
        }
    }
    if let Some(storage) = registry.get_cpu::<BoxGravity>() {
        for (&entity, solid) in storage.iter() {
            sources.push(Source {
                position: position_of(entity),
                kind: Kind::Solid {
                    settings: *solid,
                    local: local_space(matrix_of(entity)),
                },
            });
        }
    }
    sources
}

/// Splits a transform into what a local-space field needs.
///
/// The rotation alone, not the whole matrix: rotating a direction by a
/// scaled matrix would scale the acceleration, and a stretched zone is not
/// a stronger one.
fn local_space(matrix: glam::Mat4) -> LocalSpace {
    LocalSpace {
        to_local: matrix.inverse(),
        rotation: matrix.to_scale_rotation_translation().1,
    }
}

/// Applies the summed field to every dynamic body.
///
/// # Why an impulse and not a force
///
/// Rapier's forces persist across steps until `reset_forces`, so applying
/// gravity every step as a force would accumulate — the pull growing each
/// second. Resetting instead would erase whatever the game applied.
///
/// An impulse of `mass × acceleration × dt` is instantaneous, exactly
/// equivalent to that force over the step, and composes with gameplay
/// rather than fighting it.
///
/// # And why the world's own gravity is switched off
///
/// A body reached by a source has its rapier gravity scale set to zero at
/// build time, or the global vector would apply on top of the field and a
/// planet would pull diagonally.
pub fn apply_gravity_sources(resources: &mut Resources) {
    let sources = collect_sources(resources);
    if sources.is_empty() {
        return;
    }
    let dt = resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(1.0 / 60.0);

    // A source that moved, appeared or changed has to reach bodies that
    // have already settled, or switching a gravity zone on would do
    // nothing to the crates already lying in it.
    let revision = digest(&sources);
    let changed = resources
        .get::<GravityRevision>()
        .map(|previous| previous.0 != revision)
        .unwrap_or(true);
    resources.insert(GravityRevision(revision));

    let Some(mut world) = resources.remove::<kooch_physics::plugin::PhysicsWorld>() else {
        return;
    };

    // The per-body scale from phase A, read straight from storage rather
    // than copied into a map first: rapier's own gravity is off while
    // sources exist, so its `gravity_scale` would otherwise multiply
    // nothing.
    let bodies = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_physics::components::RigidBody>());

    // Collected first: reading the pose borrows the world, and applying
    // the impulse needs it mutably.
    let pulls: Vec<(kooch_physics::BodyHandle, Vec3)> = world
        .iter()
        .filter(|(_, _, spec, _)| spec.is_dynamic())
        .filter_map(|(_, entity, _, handle)| {
            // Sleeping bodies are skipped, and this is the whole reason
            // the scene stays cheap. Rapier excludes a sleeping body from
            // the island solver — that is how a pile of settled crates
            // costs nothing — and every impulse wakes what it touches. A
            // field that pulled on all of them every step would keep the
            // entire world simulating forever, which the world vector
            // never did because rapier's own gravity wakes nothing.
            // Skipped outright rather than given an impulse it would
            // bank without being simulated.
            if world.backend().is_sleeping(handle)? && !changed {
                return None;
            }
            let (position, _) = world.backend().get_transform(handle)?;
            let mass = world.backend().mass(handle)?;
            let scale = bodies
                .and_then(|storage| storage.get(entity))
                .map(|body| body.gravity_scale)
                .unwrap_or(1.0);
            let acceleration: Vec3 = sources
                .iter()
                .map(|source| source.acceleration_at(position))
                .sum::<Vec3>()
                * scale;
            // A body outside every field is left alone rather than handed a
            // zero impulse, which would wake it for nothing.
            (acceleration != Vec3::ZERO).then_some((handle, acceleration * mass * dt))
        })
        .collect();

    // `wake` only when the field itself changed. Every other step this
    // must not rouse anything, or a resting body's sleep timer resets
    // every step and it never settles — the check above would then never
    // be true and nothing in the scene would ever sleep.
    for (handle, impulse) in pulls {
        world.backend_mut().apply_impulse(handle, impulse, changed);
    }

    resources.insert(world);
}

/// What the sources looked like last step, so a change can be noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GravityRevision(u64);

/// A hash of every source's placement and settings.
///
/// Cheaper than comparing the sources themselves and enough for the one
/// question being asked: did anything about the field change since last
/// step. A collision costs one missed wake-up, not a wrong simulation.
fn digest(sources: &[Source]) -> u64 {
    let mut hash = Fnv::new();
    for source in sources {
        hash.vec3(source.position);
        match &source.kind {
            Kind::Global(global) => hash.vec3(global.acceleration),
            Kind::Point(point) => {
                hash.f32(point.strength);
                hash.f32(point.radius);
                hash.f32(point.range);
                hash.u32(point.inverse_square as u32);
            }
            Kind::Area { settings, local } => {
                hash.vec3(settings.direction);
                hash.f32(settings.strength);
                hash.vec3(settings.half_extents);
                hash.f32(settings.falloff);
                hash.matrix(local.to_local);
            }
            Kind::Solid { settings, local } => {
                hash.vec3(settings.half_extents);
                hash.f32(settings.strength);
                hash.f32(settings.rounding);
                hash.f32(settings.range);
                hash.f32(settings.falloff);
                hash.matrix(local.to_local);
            }
        }
    }
    hash.0
}

/// FNV-1a over the bit patterns, because `f32` is not `Hash` and rounding
/// it into something that is would make a slow drift invisible.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn u32(&mut self, value: u32) {
        self.0 = (self.0 ^ value as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn vec3(&mut self, value: Vec3) {
        self.f32(value.x);
        self.f32(value.y);
        self.f32(value.z);
    }

    fn matrix(&mut self, value: glam::Mat4) {
        for column in value.to_cols_array() {
            self.f32(column);
        }
    }
}

/// Switches rapier's own gravity off while any source exists.
///
/// The two do not compose: a planet pulling towards its centre plus a
/// world vector pulling down gives a diagonal, and the author placed one
/// planet. So the moment a scene has a source, gravity comes from
/// components — including the uniform kind, which is what
/// [`GlobalGravity`] is for.
///
/// A scene with no sources is untouched and keeps the world vector it
/// always had, so adding this plugin changes nothing until something asks
/// it to.
pub fn reconcile_world_gravity_for_test(resources: &mut Resources) {
    reconcile_world_gravity(resources);
}

fn reconcile_world_gravity(resources: &mut Resources) {
    let has_sources = !collect_sources(resources).is_empty();
    let Some(world) = resources.get_mut::<kooch_physics::plugin::PhysicsWorld>() else {
        return;
    };
    let wanted = match has_sources {
        true => Vec3::ZERO,
        false => Vec3::new(0.0, -9.81, 0.0),
    };
    if world.backend().gravity() != wanted {
        if has_sources {
            tracing::info!(
                target: "kooch_gravity",
                "a gravity source exists, so the world vector is off and gravity comes \
                 from components — add a GlobalGravity entity for a uniform field",
            );
        }
        world.backend_mut().set_gravity(wanted);
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
        app.add_system(Stage::PreUpdate, reconcile_world_gravity);
        app.add_system(Stage::Physics, run_if_playing(apply_gravity_sources));
    }

    fn name(&self) -> &str {
        "GravityPlugin"
    }
}
