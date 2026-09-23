//! What undoes a remote edit: the inverse of each step, and the captures it rebuilds from.

use super::*;

impl Inverse {
    /// Folds a later edit into this one, keeping this one's before-state.
    pub(super) fn absorb(&mut self, newer: Inverse) {
        match (self, newer) {
            (
                Inverse::SetField { after, .. },
                Inverse::SetField {
                    after: newer_after, ..
                },
            ) => *after = newer_after,
            // Zip stops at the shorter of the two, which is what keeps a transform's three fields
            // merging after bookkeeping has been appended as a fourth: the rider keeps the oldest
            // state, which is the one an undo wants.
            (Inverse::Several(mine), Inverse::Several(theirs)) => {
                for (mine, theirs) in mine.iter_mut().zip(theirs) {
                    mine.absorb(theirs);
                }
            }
            // The edit is always the first element; anything after it
            // rode along.
            (Inverse::Several(mine), newer) => {
                if let Some(first) = mine.first_mut() {
                    first.absorb(newer);
                }
            }
            _ => {}
        }
    }

    /// Adds an inverse that has to be applied with this one.
    pub(super) fn attach(&mut self, rider: Inverse) {
        match self {
            Inverse::Several(mine) => mine.push(rider),
            other => {
                let edit = std::mem::replace(other, Inverse::Despawn(Vec::new()));
                *other = Inverse::Several(vec![edit, rider]);
            }
        }
    }

    /// Sends this inverse, and returns the one that reverses *it*.
    pub(crate) fn apply(
        self,
        client: &RemoteClient,
        mirror: &RemoteMirror,
        resources: &mut Resources,
    ) -> Result<Inverse, String> {
        match self {
            // Writes the file both processes read. The opposite is what
            // the corners were before this put them back, so a redo has
            // something to return to.
            Inverse::BlockShape { source, shape } => {
                let opposite = crate::block_edit::shape_for(resources, source)
                    .ok_or_else(|| "the block is not loaded".to_owned())?;
                if !crate::block_edit::set_shape(resources, source, &shape) {
                    return Err("the block is not loaded".to_owned());
                }
                crate::block_edit::announce(resources, source);
                crate::block_edit::save_source(resources, source);
                Ok(Inverse::BlockShape {
                    source,
                    shape: Box::new(opposite),
                })
            }
            Inverse::SetField {
                entity,
                component,
                field,
                before,
                after,
            } => {
                client
                    .set_field(
                        entity,
                        &component,
                        &field,
                        super::remote_edit::to_remote_value(before.clone(), mirror)?,
                    )
                    .map_err(|e| e.to_string())?;
                // Swapped, not re-read: undoing an undo is redoing.
                Ok(Inverse::SetField {
                    entity,
                    component,
                    field,
                    before: after,
                    after: before,
                })
            }
            Inverse::AddComponent { entity, state } => {
                client
                    .add_component(entity, &state.name)
                    .map_err(|e| e.to_string())?;
                for (field, value) in &state.fields {
                    let value = super::remote_edit::to_remote_value(value.clone(), mirror)?;
                    if let Err(e) = client.set_field(entity, &state.name, field, value) {
                        tracing::debug!(
                            target: "kooch_editor_core::remote_undo",
                            component = %state.name,
                            %field,
                            "the component came back without one of its values: {e}",
                        );
                    }
                }
                Ok(Inverse::RemoveComponent {
                    entity,
                    component: state.name,
                })
            }
            Inverse::RemoveComponent { entity, component } => {
                let state = local_of(mirror, entity)
                    .and_then(|local| entity_state::capture_component(resources, local, &component))
                    .ok_or_else(|| format!("{component} is not on the entity to remove"))?;
                client
                    .remove_component(entity, &component)
                    .map_err(|e| e.to_string())?;
                Ok(Inverse::AddComponent { entity, state })
            }
            Inverse::Reparent {
                entity,
                before,
                after,
            } => {
                client
                    .set_parent(entity, before)
                    .map_err(|e| e.to_string())?;
                Ok(Inverse::Reparent {
                    entity,
                    before: after,
                    after: before,
                })
            }
            Inverse::Despawn(entities) => {
                let reborn = subtrees(resources, mirror, &entities);
                for entity in &entities {
                    client.despawn(*entity).map_err(|e| e.to_string())?;
                }
                Ok(Inverse::Recreate(reborn))
            }
            Inverse::Recreate(reborn) => {
                let created = rebuild(client, mirror, &reborn)?;
                Ok(Inverse::Despawn(created))
            }
            Inverse::Several(inverses) => {
                let mut opposites = Vec::with_capacity(inverses.len());
                for inverse in inverses {
                    opposites.push(inverse.apply(client, mirror, resources)?);
                }
                // Reversed: undoing a sequence walks it backwards, or the
                // second half is undone against a world the first half has
                // already changed.
                opposites.reverse();
                Ok(Inverse::Several(opposites))
            }
        }
    }
}

/// Spawns every entry, wiring the batch's own parent links as it goes.
pub(super) fn rebuild(
    client: &RemoteClient,
    mirror: &RemoteMirror,
    reborn: &[Reborn],
) -> Result<Vec<EntityId>, String> {
    let mut created: Vec<EntityId> = Vec::with_capacity(reborn.len());
    for entry in reborn {
        // Named at spawn: parenting afterwards preserves the world pose by rewriting the local
        // one, and an entity coming back from a despawn keeps the transform it was captured with.
        let parent = match entry.parent {
            Some(Ancestor::Existing(id)) => Some(id),
            Some(Ancestor::Batch(index)) => created.get(index).copied(),
            None => None,
        };
        let id = super::remote_edit::build(client, mirror, &entry.state, parent, None)?;
        created.push(id);
    }
    Ok(created)
}

/// Captures `roots` and everything under them, parents before children.
pub(crate) fn subtrees(
    resources: &Resources,
    mirror: &RemoteMirror,
    roots: &[EntityId],
) -> Vec<Reborn> {
    let mut out: Vec<Reborn> = Vec::new();
    let mut ids: Vec<EntityId> = Vec::new();
    for root in roots {
        let parent = current_parent(mirror, resources, *root).map(Ancestor::Existing);
        capture_into(resources, mirror, *root, parent, &mut out, &mut ids);
    }
    out
}

pub(super) fn capture_into(
    resources: &Resources,
    mirror: &RemoteMirror,
    id: EntityId,
    parent: Option<Ancestor>,
    out: &mut Vec<Reborn>,
    ids: &mut Vec<EntityId>,
) {
    let Some(local) = mirror.local_of(id) else {
        return;
    };
    let index = out.len();
    out.push(Reborn {
        state: entity_state::capture(resources, local),
        parent,
    });
    ids.push(id);

    for child in children_of(resources, mirror, local) {
        capture_into(
            resources,
            mirror,
            child,
            Some(Ancestor::Batch(index)),
            out,
            ids,
        );
    }
}

/// The remote ids of `local`'s direct children, read off the mirror.
pub(super) fn children_of(
    resources: &Resources,
    mirror: &RemoteMirror,
    local: kooch_ecs::entity::Entity,
) -> Vec<EntityId> {
    let Some(storage) = resources
        .get::<kooch_ecs::component::ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_ecs::hierarchy::Parent>())
    else {
        return Vec::new();
    };
    storage
        .iter()
        .filter(|(_, parent)| parent.entity == local)
        .filter_map(|(child, _)| mirror.remote_of(*child))
        .collect()
}

/// The parent the project currently has for `entity`, read off the mirror.
pub(super) fn current_parent(
    mirror: &RemoteMirror,
    resources: &Resources,
    entity: EntityId,
) -> Option<EntityId> {
    let local = mirror.local_of(entity)?;
    let parent = resources
        .get::<kooch_ecs::component::ComponentRegistry>()?
        .get_cpu::<kooch_ecs::hierarchy::Parent>()?
        .get(local)?
        .entity;
    mirror.remote_of(parent)
}

/// One field's current value, read off the mirror.
pub(super) fn field_value(
    resources: &Resources,
    entity: kooch_ecs::entity::Entity,
    component: &str,
    field: &str,
) -> Option<ReflectValue> {
    entity_state::capture_component(resources, entity, component)?
        .fields
        .into_iter()
        .find(|(name, _)| name == field)
        .map(|(_, value)| value)
}

pub(super) fn local_of(
    mirror: &RemoteMirror,
    entity: EntityId,
) -> Option<kooch_ecs::entity::Entity> {
    mirror.local_of(entity)
}
