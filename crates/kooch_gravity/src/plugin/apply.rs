//! Handing the summed field to the solver, and keeping it from fighting
//! rapier's own gravity.

use glam::{Mat4, Vec3};

use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::component::ComponentRegistry;

use super::collect::{Kind, Source, collect_sources};

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
    let field = collect_sources(resources);
    if field.is_empty() {
        return;
    }
    let dt = resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(1.0 / 60.0);

    // A source that moved, appeared or changed has to reach bodies that
    // have already settled, or switching a gravity zone on would do
    // nothing to the crates already lying in it.
    let revision = digest(&field.sources);
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
        .and_then(|registry| registry.get_cpu::<kooch_physics::components::PhysicsBody>());

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
            let acceleration = field.acceleration_at(position) * scale;
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

/// A hash of every source's placement, level and settings.
///
/// Cheaper than comparing the sources themselves and enough for the one
/// question being asked: did anything about the field change since last
/// step. A collision costs one missed wake-up, not a wrong simulation.
fn digest(sources: &[Source]) -> u64 {
    let mut hash = Fnv::new();
    for source in sources {
        hash.vec3(source.position);
        // A zone that stops overruling has to wake what it was holding up.
        hash.u32(source.level as u32);
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
            Kind::Plane { settings, local } => {
                hash.vec3(settings.normal);
                hash.f32(settings.strength);
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

    fn matrix(&mut self, value: Mat4) {
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
/// [`GlobalGravity`](crate::GlobalGravity) is for.
///
/// A scene with no sources is untouched and keeps the world vector it
/// always had, so adding this plugin changes nothing until something asks
/// it to.
pub fn reconcile_world_gravity_for_test(resources: &mut Resources) {
    reconcile_world_gravity(resources);
}

pub(super) fn reconcile_world_gravity(resources: &mut Resources) {
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
