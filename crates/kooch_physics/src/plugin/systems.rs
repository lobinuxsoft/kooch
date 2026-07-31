//! The three systems that make a cube fall: reconcile, step, write back.
//!
//! # Where they run
//!
//! ```text
//! PreUpdate    physics_sync_system        once per frame
//! Physics      physics_step_system        once per fixed step, playing only
//! PostPhysics  physics_writeback_system   once per fixed step, playing only
//! ```
//!
//! `Physics` and `PostPhysics` are the engine's fixed-timestep stages:
//! [`Time`] accumulates the frame's delta and the runner runs them however
//! many whole steps have built up, so the result is the same at 30 and at
//! 144 fps. They sit between `Update` and `PostUpdate`, which is what makes
//! writeback here reach transform propagation and the GPU upload in the
//! same frame instead of the next one.

use glam::{Quat, Vec3};

use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_core::time::Time;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::transform::Transform;

use crate::components::{Collider, RigidBody};

use super::world::{BodySpec, PhysicsBody, PhysicsWorld};

/// Fallback step when the app has no [`Time`] — 60 Hz.
const FALLBACK_DT: f32 = 1.0 / 60.0;

/// How far a pose may drift before the authored transform is pushed into
/// the solver. Below this, pushing would only wake sleeping bodies.
const POSE_EPSILON: f32 = 1e-5;

/// What the ECS says an entity's body should be, this frame.
struct Authored {
    entity: Entity,
    /// Shapes contributed by descendants that have no body of their own.
    attachments: Vec<super::compound::Attachment>,
    /// The slot the entity currently claims, if it carries a
    /// [`PhysicsBody`] already.
    claimed: Option<u32>,
    spec: BodySpec,
    position: Vec3,
    rotation: Quat,
}

/// Reconciles the backend's bodies with the authored components.
///
/// Runs every frame, playing or not: the body set mirrors the ECS while
/// authoring too, so a scene loaded in the editor is immediately ready to
/// simulate and immediately answerable to scene queries.
///
/// Three passes, in order:
///
/// 1. **Retire** — slots whose entity no longer claims them (despawned,
///    lost its `RigidBody`, or came back from a stop without the runtime
///    component) and slots whose spec no longer matches the Inspector.
/// 2. **Create** — entities with a `RigidBody` and no live slot.
/// 3. **Push** — authored poses into the solver: every body while
///    authoring, kinematic bodies always.
pub fn physics_sync_system(resources: &mut Resources) {
    let Some(mut world) = resources.remove::<PhysicsWorld>() else {
        return;
    };
    let playing = Playing::is_playing(resources);

    let Some(mut authored) = read_authored(resources) else {
        resources.insert(world);
        return;
    };

    retire_stale_slots(&mut world, &authored);
    let gained = create_missing_bodies(resources, &mut world, &mut authored);
    // Last, not first: retiring frees slots and creating re-points the
    // components that still have a body. Whatever is left pointing at
    // nothing is a genuine orphan.
    let orphans = find_orphans(resources, &world);
    push_authored_poses(&mut world, &authored, playing);
    // After the bodies exist: a joint resolves its two entity references
    // through the slots this pass just wrote, so running it first would
    // leave every joint in a scene's first frame waiting on bodies that
    // are about to appear.
    super::joints::sync_joints(resources, &mut world);

    resources.insert(world);
    sync_archetypes(resources, &gained, &orphans);
}

/// Reads the authored intent, in a deterministic order.
///
/// Returns `None` when there is no component registry to read.
fn read_authored(resources: &Resources) -> Option<Vec<Authored>> {
    let registry = resources.get::<ComponentRegistry>()?;
    let slots = registry.get_cpu::<PhysicsBody>();

    let bodies = registry.get_cpu::<RigidBody>();
    let colliders = registry.get_cpu::<Collider>();
    let transforms = registry.get_cpu::<Transform>();

    let mut authored: Vec<Authored> = bodies
        .map(|storage| {
            storage
                .iter()
                .map(|(&entity, body)| {
                    // A half-authored entity gets defaults rather than a
                    // panic: a unit sphere at the origin still falls, and
                    // falling is a better bug report than a crash.
                    let collider = collider_or_default(colliders, entity);
                    let transform = transforms
                        .and_then(|s| s.get(entity))
                        .copied()
                        .unwrap_or_default();
                    let attachments = super::compound::attachments_for(resources, entity);
                    Authored {
                        entity,
                        claimed: slots.and_then(|s| s.get(entity)).map(PhysicsBody::slot),
                        spec: BodySpec::with_attachments(
                            body,
                            &collider,
                            transform.scale,
                            super::compound::digest(&attachments),
                        ),
                        position: transform.position,
                        rotation: transform.rotation,
                        attachments,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Component storage is a hash map, so its iteration order varies
    // between runs. Body creation order is observable in the solver's
    // output, so fix it to entity order — otherwise two runs of the same
    // scene diverge for no reason anyone can see.
    authored.sort_unstable_by_key(|a| (a.entity.index(), a.entity.generation()));

    Some(authored)
}

/// Entities carrying a [`PhysicsBody`] with nothing behind it — their
/// `RigidBody` went away, or the slot they name belongs to someone else
/// now. The component has to follow the body it addressed.
fn find_orphans(resources: &Resources, world: &PhysicsWorld) -> Vec<Entity> {
    resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<PhysicsBody>())
        .map(|storage| {
            storage
                .iter()
                .filter(|(entity, body)| world.entity(body.slot()) != Some(**entity))
                .map(|(&entity, _)| entity)
                .collect()
        })
        .unwrap_or_default()
}

/// The entity's collider, or the default unit sphere.
fn collider_or_default(
    colliders: Option<&kooch_ecs::component::ComponentStorage<Collider>>,
    entity: Entity,
) -> Collider {
    colliders
        .and_then(|s| s.get(entity))
        .copied()
        .unwrap_or_default()
}

/// Frees slots nothing claims any more, and slots whose spec went stale.
fn retire_stale_slots(world: &mut PhysicsWorld, authored: &[Authored]) {
    // A slot survives only if the entity that owns it still declares a
    // body with the same spec. Everything else — despawns, removed
    // components, a stop that wiped the runtime component, an Inspector
    // edit that changed the shape — retires it.
    let mut keep = vec![false; world.capacity()];
    for entry in authored {
        let Some(slot) = entry.claimed else { continue };
        if world.entity(slot) == Some(entry.entity) && world.spec(slot) == Some(entry.spec) {
            keep[slot as usize] = true;
        }
    }
    for slot in 0..world.capacity() as u32 {
        if !keep[slot as usize] {
            world.remove(slot);
        }
    }
}

/// Creates bodies for entities without a live slot, returning the
/// entities that gained a [`PhysicsBody`] component.
fn create_missing_bodies(
    resources: &mut Resources,
    world: &mut PhysicsWorld,
    authored: &mut [Authored],
) -> Vec<Entity> {
    let mut gained = Vec::new();
    for entry in authored {
        if entry
            .claimed
            .is_some_and(|slot| world.entity(slot) == Some(entry.entity))
        {
            continue;
        }
        let slot = world.insert(entry.entity, entry.spec, entry.position, entry.rotation);
        // Shapes inherited from descendants join the body it just built.
        // Attached here rather than in `world.insert` because they are
        // gathered from the ECS, and `PhysicsWorld` deliberately knows
        // nothing about entities beyond the one that owns each slot.
        world.attach_all(slot, &entry.attachments);
        // Point the entry at its new slot so the pose pass does not have
        // to go looking for it — a search there would be quadratic in the
        // number of bodies every time a scene loads.
        entry.claimed = Some(slot);

        let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
            continue;
        };
        let Some(storage) = registry.get_cpu_mut::<PhysicsBody>() else {
            continue;
        };
        // Only a first insertion changes the archetype; re-pointing an
        // entity that already carried the component does not.
        if storage
            .insert(entry.entity, PhysicsBody::new(slot))
            .is_none()
        {
            gained.push(entry.entity);
        }
    }
    gained
}

/// Pushes authored poses into the solver.
///
/// While authoring, every body follows its `Transform` — dragging a gizmo
/// has to move the collider, or the next Play starts from a world the
/// solver has never seen. While playing, only kinematic bodies do; the
/// solver owns dynamic poses and pushing them would fight it.
fn push_authored_poses(world: &mut PhysicsWorld, authored: &[Authored], playing: bool) {
    for entry in authored {
        if playing && !entry.spec.is_kinematic() {
            continue;
        }
        let Some(slot) = entry.claimed else { continue };
        let Some(handle) = world.handle(slot) else {
            continue;
        };
        // Setting a pose republishes AABBs and wakes the body, so skip it
        // when nothing moved — which is every frame, for most bodies.
        if let Some((position, rotation)) = world.backend().get_transform(handle)
            && position.abs_diff_eq(entry.position, POSE_EPSILON)
            && rotation.abs_diff_eq(entry.rotation, POSE_EPSILON)
        {
            continue;
        }
        world
            .backend_mut()
            .set_transform(handle, entry.position, entry.rotation);
    }
}

/// Moves entities between archetypes after the component churn.
///
/// Without this the component sits in storage but no archetype lists it,
/// so every query iterating by archetype misses it.
fn sync_archetypes(resources: &mut Resources, gained: &[Entity], orphans: &[Entity]) {
    if !orphans.is_empty() {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<PhysicsBody>()
        {
            for &entity in orphans {
                storage.remove(entity);
            }
        }
    }

    if gained.is_empty() && orphans.is_empty() {
        return;
    }
    let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() else {
        return;
    };
    for &entity in gained {
        if let Some(current) = archetypes.entity_archetype(entity) {
            let next = archetypes.archetype_after_add::<PhysicsBody>(current);
            archetypes.register_entity(entity, next);
        }
    }
    for &entity in orphans {
        if let Some(current) = archetypes.entity_archetype(entity) {
            let next = archetypes.archetype_after_remove::<PhysicsBody>(current);
            archetypes.register_entity(entity, next);
        }
    }
}

/// Advances the simulation by one fixed step.
///
/// Registered in [`Stage::Physics`], which the runner runs once per whole
/// step accumulated in [`Time`] — so the step size never depends on the
/// frame rate.
///
/// [`Stage::Physics`]: kooch_core::stage::Stage::Physics
pub fn physics_step_system(resources: &mut Resources) {
    let dt = resources
        .get::<Time>()
        .map(|time| time.fixed_delta_secs())
        .unwrap_or(FALLBACK_DT);
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        world.backend_mut().step(dt);
        // Immediately after the step that produced them, so a joint that
        // broke is already gone from the registry before anything reads it.
        super::joints::collect_broken_joints(world);
    }
}

/// Copies solver poses back onto `Transform`.
///
/// Dynamic bodies only: a static body never moves, and a kinematic one is
/// driven *from* its `Transform`, so writing back would erase the frame's
/// authored motion.
pub fn physics_writeback_system(resources: &mut Resources) {
    let Some(world) = resources.remove::<PhysicsWorld>() else {
        return;
    };

    let poses: Vec<(Entity, Vec3, Quat)> = world
        .iter()
        .filter(|(_, _, spec, _)| spec.is_dynamic())
        .filter_map(|(_, entity, _, handle)| {
            let (position, rotation) = world.backend().get_transform(handle)?;
            Some((entity, position, rotation))
        })
        .collect();

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
    {
        for (entity, position, rotation) in poses {
            if let Some(transform) = storage.get_mut(entity) {
                transform.position = position;
                transform.rotation = rotation;
            }
        }
    }

    resources.insert(world);
}
