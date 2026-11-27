//! Bodies kept out of the simulation until something they stand on exists, such as a planet's
//! ground colliders (#1949). Held, a body keeps its pose and velocity instead of falling through.

use std::collections::{HashMap, HashSet};

use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::backend::BodyHandle;

use super::world::{PhysicsWorld, SolverBody};

/// Who holds bodies replaces its whole set each frame with [`PhysicsHolds::set`]; the step only
/// touches the bodies whose state changed.
#[derive(Default)]
pub struct PhysicsHolds {
    wanted: HashSet<Entity>,
    /// What the solver was told, by the handle it was told on: a rebuilt body comes back enabled.
    applied: HashMap<Entity, BodyHandle>,
}

impl PhysicsHolds {
    /// The bodies to hold from the next step on; any other is released.
    pub fn set(&mut self, held: impl IntoIterator<Item = Entity>) {
        self.wanted.clear();
        self.wanted.extend(held);
    }

    /// Whether `entity` is asked to be held.
    pub fn holds(&self, entity: Entity) -> bool {
        self.wanted.contains(&entity)
    }
}

/// Disables what is newly held and enables what was released, before the step.
pub(super) fn apply(resources: &mut Resources) {
    let Some(mut holds) = resources.remove::<PhysicsHolds>() else {
        return;
    };
    if holds.wanted.is_empty() && holds.applied.is_empty() {
        resources.insert(holds);
        return;
    }
    let handles: Vec<(Entity, Option<BodyHandle>)> = {
        let solver = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<SolverBody>());
        let world = resources.get::<PhysicsWorld>();
        let handle = |entity: Entity| {
            let slot = solver.as_ref()?.get(entity)?.slot();
            world.as_ref()?.handle(slot)
        };
        holds
            .wanted
            .iter()
            .chain(holds.applied.keys())
            .map(|&entity| (entity, handle(entity)))
            .collect()
    };
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let backend = world.backend_mut();
        for (entity, handle) in handles {
            let wanted = holds.wanted.contains(&entity);
            let applied = holds.applied.get(&entity).copied();
            match (wanted, handle) {
                (true, Some(handle)) if applied != Some(handle) => {
                    backend.set_enabled(handle, false);
                    holds.applied.insert(entity, handle);
                }
                (false, handle) => {
                    if let Some(handle) = handle
                        && applied == Some(handle)
                    {
                        backend.set_enabled(handle, true);
                    }
                    holds.applied.remove(&entity);
                }
                (true, None) => {
                    holds.applied.remove(&entity);
                }
                _ => {}
            }
        }
    }
    resources.insert(holds);
}
