//! Reconciling authored [`Joint`] components with the solver's joints.
//!
//! # Why joints are not addressed by a slot component
//!
//! Bodies are: a [`PhysicsBody`] carries the slot, because both directions
//! of the mapping are walked every frame — sync asks "does this entity have
//! a body", writeback asks "which entity owns this body". A joint has no
//! writeback. Nothing reads a joint back onto the ECS, so the reverse
//! direction never happens and the component would be bookkeeping nobody
//! queries.
//!
//! What is left is entity → joint, once per frame, over a set far smaller
//! than the body set. A map is the honest shape for that.
//!
//! # How a joint knows to rebuild
//!
//! Not by comparing itself to the solver, but by remembering the two
//! [`BodyHandle`]s it was built from. A body handle changes whenever its
//! body is rebuilt — an Inspector edit, a scale change, and crucially a
//! stop, which drops every [`PhysicsBody`] and rebuilds the world from the
//! restored ECS. So "my bodies' handles moved" already means everything
//! "the play session ended" would have to mean, and the joint set follows
//! the body set without a second lifecycle to keep in step.
//!
//! [`PhysicsBody`]: super::world::PhysicsBody

use std::collections::{HashMap, HashSet};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::EntityRef;

use crate::backend::{BodyHandle, JointDesc, JointHandle};
use crate::components::Joint;

use super::events::JointBroke;

use super::world::{PhysicsBody, PhysicsWorld};

/// One authored joint, and what the solver made of it.
struct JointSlot {
    /// The component as authored, so an Inspector edit is a rebuild.
    spec: Joint,
    /// The bodies it was built from. These moving means the bodies were
    /// rebuilt, and a joint into a dead handle holds nothing.
    bodies: (BodyHandle, BodyHandle),
    /// The entities those bodies belong to.
    ///
    /// Kept rather than re-derived from [`Self::spec`], because a broken
    /// joint has to name them and the spec holds references, which resolve
    /// to entities only while their targets are loaded. A joint the solver
    /// built had both; that fact should not have to be rediscovered at
    /// report time, where failing would be unreportable.
    targets: (Entity, Entity),
    /// The live joint, or `None` when the backend refused to build it or
    /// it broke under load.
    ///
    /// `None` is deliberately sticky: a joint that broke must not come back
    /// the next frame, and the spec has not changed, so nothing here asks
    /// for it. Pressing stop rebuilds the bodies, which moves the handles,
    /// which rebuilds the joint.
    joint: Option<JointHandle>,
}

/// The authored joints, keyed by the entity carrying each one.
#[derive(Default)]
pub struct JointRegistry {
    slots: HashMap<Entity, JointSlot>,
    /// Joints already complained about, so an unresolvable reference is
    /// one log line rather than one per frame forever.
    warned: HashSet<Entity>,
    /// Breaks waiting to be reported. #560 built the breaking with nowhere
    /// to send it; #561 is the somewhere.
    breaks: Vec<JointBroke>,
}

impl JointRegistry {
    /// Number of authored joints being tracked, built or not.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// `true` when nothing is tracked.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The breaks collected since the last drain, for the event pass.
    pub(super) fn drained_breaks(&mut self) -> &mut Vec<JointBroke> {
        &mut self.breaks
    }

    /// Whether the joint authored on `entity` is currently live in the
    /// solver. `false` covers "not built yet", "refused" and "broke".
    pub fn is_built(&self, entity: Entity) -> bool {
        self.slots
            .get(&entity)
            .is_some_and(|slot| slot.joint.is_some())
    }
}

/// What the ECS says a joint should be, this frame.
struct Authored {
    entity: Entity,
    spec: Joint,
    /// The bodies both references resolve to, or `None` while either is
    /// still unresolved.
    bodies: Option<(BodyHandle, BodyHandle)>,
    /// The entities behind [`Self::bodies`], resolved in the same pass.
    targets: Option<(Entity, Entity)>,
}

/// Reconciles the solver's joints with the authored [`Joint`] components.
///
/// Runs after bodies are reconciled, and for the same reason bodies are
/// reconciled every frame rather than on play: a scene loaded in the editor
/// should already hold together.
pub(super) fn sync_joints(resources: &Resources, world: &mut PhysicsWorld) {
    let authored = read_authored(resources, world);
    retire_stale_joints(world, &authored);
    build_missing_joints(world, &authored);
}

/// Reads the authored joints, resolving both entity references to bodies.
///
/// Deterministic order for the same reason body creation is: joint
/// insertion order is observable in the solver, and component storage is a
/// hash map whose iteration order varies between runs.
fn read_authored(resources: &Resources, world: &PhysicsWorld) -> Vec<Authored> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(joints) = registry.get_cpu::<Joint>() else {
        return Vec::new();
    };
    let slots = registry.get_cpu::<PhysicsBody>();

    // An unresolved reference yields no body, exactly like a missing one:
    // the target's scene is not open yet, so the joint waits rather than
    // being built against the wrong entity.
    let resolve = |reference: Option<EntityRef>| -> Option<(Entity, BodyHandle)> {
        let entity = reference?.entity()?;
        let slot = slots?.get(entity)?.slot();
        // Both checks matter: the slot could have been recycled by another
        // entity since this component last looked at it.
        (world.entity(slot) == Some(entity))
            .then(|| world.handle(slot))
            .flatten()
            .map(|handle| (entity, handle))
    };

    let mut authored: Vec<Authored> = joints
        .iter()
        .map(|(&entity, spec)| {
            let pair = resolve(spec.body_a).zip(resolve(spec.body_b));
            Authored {
                entity,
                spec: *spec,
                bodies: pair.map(|((_, a), (_, b))| (a, b)),
                targets: pair.map(|((a, _), (b, _))| (a, b)),
            }
        })
        .collect();
    authored.sort_unstable_by_key(|a| (a.entity.index(), a.entity.generation()));
    authored
}

/// Drops joints whose component went away, whose bodies were rebuilt, or
/// whose parameters changed.
fn retire_stale_joints(world: &mut PhysicsWorld, authored: &[Authored]) {
    let keep: HashMap<Entity, (Joint, (BodyHandle, BodyHandle))> = authored
        .iter()
        .filter_map(|entry| Some((entry.entity, (entry.spec, entry.bodies?))))
        .collect();

    let stale: Vec<Entity> = world
        .joints()
        .slots
        .iter()
        .filter(|(entity, slot)| {
            keep.get(entity)
                .is_none_or(|(spec, bodies)| *spec != slot.spec || *bodies != slot.bodies)
        })
        .map(|(&entity, _)| entity)
        .collect();

    for entity in stale {
        if let Some(slot) = world.joints_mut().slots.remove(&entity)
            && let Some(handle) = slot.joint
        {
            world.backend_mut().remove_joint(handle);
        }
        world.joints_mut().warned.remove(&entity);
    }
}

/// Builds the joints that have no live slot yet.
fn build_missing_joints(world: &mut PhysicsWorld, authored: &[Authored]) {
    for entry in authored {
        if world.joints().slots.contains_key(&entry.entity) {
            continue;
        }
        let (Some(bodies), Some(targets)) = (entry.bodies, entry.targets) else {
            warn_unresolved(world, entry);
            continue;
        };
        world.joints_mut().warned.remove(&entry.entity);

        let joint = world.backend_mut().add_joint(desc_for(&entry.spec, bodies));
        world.joints_mut().slots.insert(
            entry.entity,
            JointSlot {
                spec: entry.spec,
                bodies,
                targets,
                joint,
            },
        );
    }
}

/// Says once that a joint names something it cannot reach.
///
/// Not an error: a reference into a scene that is not resident is the
/// normal state under streaming, and a joint whose partner has not spawned
/// yet has to wait rather than be dropped. It is still worth saying,
/// because the other cause — an entity named in the Inspector that has no
/// `RigidBody` — looks identical from here and is a genuine mistake.
fn warn_unresolved(world: &mut PhysicsWorld, entry: &Authored) {
    if !world.joints_mut().warned.insert(entry.entity) {
        return;
    }
    tracing::warn!(
        target: "kooch_physics",
        entity = entry.entity.index(),
        "a Joint is waiting for its bodies — both Body A and Body B have to name \
         entities that carry a RigidBody. The joint holds nothing until they do",
    );
}

/// Builds the backend descriptor from the authored component.
fn desc_for(spec: &Joint, (body_a, body_b): (BodyHandle, BodyHandle)) -> JointDesc {
    let kind = spec.joint_kind();
    let has_axis = kind.has_primary_axis();
    JointDesc {
        body_a,
        body_b,
        kind,
        anchor_a: spec.anchor_a,
        anchor_b: spec.anchor_b,
        // A limit or a motor on a kind with no free axis is not silently
        // dropped by the backend — it is not passed, so the two layers
        // cannot disagree about which one enforces the rule.
        limits: has_axis.then(|| spec.limits()).flatten(),
        motor: has_axis.then(|| spec.motor()).flatten(),
        articulated: spec.articulated,
        contacts_enabled: spec.contacts_enabled,
        break_impulse: spec.break_impulse(),
    }
}

/// Removes the joints that broke during the last step.
///
/// The slot stays, with no joint in it: the component is still authored, so
/// forgetting the slot would have the next sync build the joint again and
/// break it again, forever. See [`JointSlot::joint`].
pub(super) fn collect_broken_joints(world: &mut PhysicsWorld) {
    let broken = world.backend_mut().take_broken_joints();
    if broken.is_empty() {
        return;
    }
    let dead: HashMap<JointHandle, f32> = broken
        .iter()
        .map(|event| (event.joint, event.impulse))
        .collect();
    let mut reported = Vec::new();
    for (&entity, slot) in world.joints_mut().slots.iter_mut() {
        let Some(impulse) = slot.joint.and_then(|handle| dead.get(&handle).copied()) else {
            continue;
        };
        slot.joint = None;
        reported.push((entity, slot.targets.0, slot.targets.1, impulse));
        tracing::info!(
            target: "kooch_physics",
            entity = entity.index(),
            "a joint broke under load",
        );
    }
    let registry = world.joints_mut();
    for (joint, a, b, impulse) in reported {
        registry.breaks.push(JointBroke {
            joint,
            a,
            b,
            impulse,
        });
    }
}
