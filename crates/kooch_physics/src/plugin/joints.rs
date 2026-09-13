//! Reconciles authored [`Joint`]s with an entity → joint map (nothing reads joints back). A joint
//! rebuilds when its [`BodyHandle`]s change, which already covers edits and stop.

use std::collections::{HashMap, HashSet};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::EntityRef;

use crate::backend::{BodyHandle, JointDesc, JointHandle};
use crate::components::Joint;

use super::events::JointBroke;

use super::world::{PhysicsWorld, SolverBody};

/// One authored joint, and what the solver made of it.
struct JointSlot {
    /// The component as authored, so an Inspector edit is a rebuild.
    spec: Joint,
    /// The bodies it was built from. These moving means the bodies were
    /// rebuilt, and a joint into a dead handle holds nothing.
    bodies: (BodyHandle, BodyHandle),
    /// The bodies' entities, kept because a broken joint must name them and the spec's references
    /// only resolve while loaded.
    targets: (Entity, Entity),
    /// The live joint, `None` when refused or broken — sticky, so a broken joint stays broken until
    /// stop rebuilds the bodies.
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

/// Reconciles joints after bodies, every frame, so a scene loaded in the editor already holds
/// together.
pub(super) fn sync_joints(resources: &Resources, world: &mut PhysicsWorld) {
    let authored = read_authored(resources, world);
    retire_stale_joints(world, &authored);
    build_missing_joints(world, &authored);
}

/// Authored joints with both references resolved, in deterministic order: insertion order is
/// observable and storage is a hash map.
fn read_authored(resources: &Resources, world: &PhysicsWorld) -> Vec<Authored> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(joints) = registry.get_cpu::<Joint>() else {
        return Vec::new();
    };
    let slots = registry.get_cpu::<SolverBody>();

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

/// Warns once about an unreachable reference: normal while streaming, but also what an entity
/// without `PhysicsBody` looks like.
fn warn_unresolved(world: &mut PhysicsWorld, entry: &Authored) {
    if !world.joints_mut().warned.insert(entry.entity) {
        return;
    }
    tracing::warn!(
        target: "kooch_physics",
        entity = entry.entity.index(),
        "a Joint is waiting for its bodies — both Body A and Body B have to name \
         entities that carry a PhysicsBody. The joint holds nothing until they do",
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

/// Removes broken joints but keeps the slot, or the next sync rebuilds and breaks it forever. See
/// [`JointSlot::joint`].
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
