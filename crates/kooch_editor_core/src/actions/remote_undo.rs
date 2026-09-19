//! Undo and redo while the project owns the world.
//!
//! # The bug this exists to fix
//!
//! ```text
//! // remote_edit.rs, until this module
//! if matches!(action, EditorAction::Undo | EditorAction::Redo) { return true; }
//! ```
//!
//! Opening a project puts the editor in remote mode for as long as it is
//! open, so that line was every Ctrl+Z anyone pressed. The
//! [`UndoStack`](crate::undo::UndoStack) it should have reached is fine
//! and full of commands — it just describes the **mirror**, and replaying
//! a command against the mirror is undone by the next refresh half a
//! second later. Swallowing was the honest thing to do; leaving it
//! swallowed was not.
//!
//! # How this one works instead
//!
//! Not commands: **inverses**. Before an edit goes out, the editor asks
//! what the world looked like and keeps the edit that would put it back.
//! Undo sends that inverse down the same wire as any other edit, and the
//! mirror catches up on the next refresh like it does for everything
//! else.
//!
//! [`Inverse::apply`] returns *its own* inverse, which is what makes redo
//! fall out for free: undoing pushes onto the redo stack, redoing pushes
//! back onto the undo stack, and neither direction needs its own code
//! path.
//!
//! # Why remote ids and not mirror handles
//!
//! Every id here is an [`EntityId`] — the project's own handle. The
//! mirror's [`Entity`] is a local stand-in whose only guarantee is that it
//! survives a refresh; an entity that goes away and comes back gets a new
//! one. The project's id is the identity the *other* process agrees on,
//! which is the only thing worth writing in a history that outlives the
//! edit.

use kooch_core::resource::Resources;
use kooch_ecs::reflect::ReflectValue;
use kooch_remote::RemoteClient;
use kooch_remote::protocol::EntityId;

use crate::remote_mirror::RemoteMirror;

use super::EditorAction;
use super::entity_state::{self, ComponentState, EntityState};

/// How deep the history goes, matching the local [`UndoStack`](crate::undo::UndoStack).
const DEPTH: usize = 100;

/// The edit that puts the world back the way it was.
pub(crate) enum Inverse {
    /// Both sides of a field edit.
    SetField {
        entity: EntityId,
        component: String,
        field: String,
        before: ReflectValue,
        after: ReflectValue,
    },
    /// Put a component back, with the values it had.
    AddComponent {
        entity: EntityId,
        state: ComponentState,
    },
    RemoveComponent {
        entity: EntityId,
        component: String,
    },
    /// Both sides of a reparent, for the same reason [`Inverse::SetField`]
    /// carries both.
    Reparent {
        entity: EntityId,
        before: Option<EntityId>,
        after: Option<EntityId>,
    },
    /// Undo of anything that created entities.
    Despawn(Vec<EntityId>),
    /// Undo of a despawn — the whole subtree, since that is what the
    /// project's despawn took (`kooch_remote::handlers`, "Despawns an
    /// entity **and everything under it**").
    Recreate(Vec<Reborn>),
    /// A block's corners, as they were.
    BlockShape {
        source: kooch_core::Guid,
        shape: Box<kooch_blockmesh::BlockMesh>,
    },
    /// Several edits that have to travel together, applied in order.
    Several(Vec<Inverse>),
}

/// One entity to bring back, and where it hung.
pub(crate) struct Reborn {
    pub state: EntityState,
    pub parent: Option<Ancestor>,
}

/// A parent that either still exists or is being recreated alongside.
pub(crate) enum Ancestor {
    Existing(EntityId),
    Batch(usize),
}

/// One reversible thing the user did, with a name they would recognise.
pub(crate) struct Step {
    pub label: String,
    pub inverse: Inverse,
    /// What this edit was aimed at, so a run of them can be recognised
    /// as one. `None` for a discrete edit — a spawn is never half of
    /// something bigger.
    pub key: Option<crate::history::MergeKey>,
}

/// The remote counterpart of [`UndoStack`](crate::undo::UndoStack).
#[derive(Default)]
pub(crate) struct RemoteHistory {
    done: Vec<Step>,
    undone: Vec<Step>,
    /// Set when something closed the current run of edits — see
    /// [`crate::history::merge`].
    sealed: bool,
}

impl RemoteHistory {
    /// Records an edit that has already been sent.
    pub fn record(&mut self, step: Step) {
        let sealed = std::mem::take(&mut self.sealed);
        // A continuation keeps the *older* step's before-state — that is what an undo has to reach
        // — and takes the newer one's after-state, which is what a redo has to write. Sixty frames
        // of a drag become one step holding where it started and where it ended.
        if crate::history::merge::continues(
            self.done.last().and_then(|top| top.key),
            step.key,
            sealed,
        ) {
            let depth = self.done.len();
            if let Some(top) = self.done.last_mut() {
                tracing::debug!(
                    target: "kooch_editor_core::remote_undo",
                    label = %top.label,
                    depth,
                    "merged into the step above",
                );
                top.inverse.absorb(step.inverse);
                return;
            }
        }
        // 🔴 One line per step the history takes, because "how many steps did that edit file?" is
        // not answerable from the outside: the symptom of getting it wrong is a Ctrl+Z that needs
        // pressing twice, and by then the evidence is gone.
        tracing::debug!(
            target: "kooch_editor_core::remote_undo",
            label = %step.label,
            keyed = step.key.is_some(),
            sealed,
            depth = self.done.len() + 1,
            "new step",
        );
        self.done.push(step);
        self.undone.clear();
        while self.done.len() > DEPTH {
            self.done.remove(0);
        }
    }

    /// Folds bookkeeping into the step that caused it.
    pub fn attach(&mut self, inverse: Inverse) {
        let Some(top) = self.done.last_mut() else {
            self.done.push(Step {
                label: "Set overrides".to_owned(),
                inverse,
                key: None,
            });
            return;
        };
        tracing::debug!(
            target: "kooch_editor_core::remote_undo",
            label = %top.label,
            "bookkeeping attached to the step above",
        );
        top.inverse.attach(inverse);
    }

    /// Ends the current run of edits, so the next one starts a step.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// Forgets everything, for when the world it describes is gone —
    /// a scene load, a project close, a session that dropped.
    pub fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo_description(&self) -> Option<&str> {
        self.done.last().map(|step| step.label.as_str())
    }

    pub fn redo_description(&self) -> Option<&str> {
        self.undone.last().map(|step| step.label.as_str())
    }
}

/// Takes one step in `direction`, sending the inverse to the project.
pub(crate) fn step(resources: &mut Resources, undo: bool) -> bool {
    let Some(mut history) = resources.remove::<RemoteHistory>() else {
        return false;
    };
    let stack = match undo {
        true => &mut history.done,
        false => &mut history.undone,
    };
    let Some(step) = stack.pop() else {
        tracing::debug!(
            target: "kooch_editor_core::remote_undo",
            undo,
            "nothing left to take",
        );
        resources.insert(history);
        return false;
    };
    tracing::debug!(
        target: "kooch_editor_core::remote_undo",
        undo,
        label = %step.label,
        left = stack.len(),
        "taking a step",
    );

    // Lifted out for the same reason `remote_edit::dispatch` does it: the
    // send borrows the session while the capture reads the rest of the
    // world.
    let Some(state) = resources.remove::<crate::remote_session::RemoteState>() else {
        resources.insert(history);
        return false;
    };

    let outcome = match state.session.as_ref() {
        Some(session) => step
            .inverse
            .apply(&session.client(), &state.mirror, resources),
        None => Err("no session".to_owned()),
    };

    resources.insert(state);

    // Whether it worked or not, what the mirror shows is now a guess.
    super::remote_edit::pull_soon(resources);

    match outcome {
        Ok(inverse) => {
            let opposite = match undo {
                true => &mut history.undone,
                false => &mut history.done,
            };
            opposite.push(Step {
                label: step.label,
                inverse,
                // The opposite stack is walked one entry at a time, so a
                // step that came out of an undo never merges with
                // anything.
                key: None,
            });
        }
        // 🔴 The step is *not* put back. It described a world that no longer matches — an entity
        // someone else deleted, a component the project refused — and a history whose next step is
        // known to fail is worse than a short one, because the second Ctrl+Z would hit it again.
        Err(e) => tracing::warn!(
            target: "kooch_editor_core::remote_undo",
            label = %step.label,
            "the step could not be applied and was dropped: {e}",
        ),
    }

    resources.insert(history);
    true
}

/// Reads what an edit is about to destroy, before it is sent.
pub(crate) fn capture_before(
    action: &EditorAction,
    resources: &Resources,
    mirror: &RemoteMirror,
) -> Option<Inverse> {
    match action {
        EditorAction::SetField {
            entity,
            component,
            field,
            value,
        } => {
            let id = mirror.remote_of(*entity)?;
            let component = component_name(resources, *component)?;
            let before = field_value(resources, *entity, &component, field)?;
            Some(Inverse::SetField {
                entity: id,
                component,
                field: field.clone(),
                before,
                after: value.clone(),
            })
        }
        // The action already carries the transform it started from, which
        // is exactly what an undo has to write back — no need to read the
        // world for a value the gizmo has been holding all along.
        EditorAction::TransformEdit {
            entity,
            before,
            after,
            ..
        } => {
            let id = mirror.remote_of(*entity)?;
            let component = std::any::type_name::<kooch_ecs::transform::Transform>().to_owned();
            Some(Inverse::Several(
                [
                    (
                        "position",
                        ReflectValue::Vec3(before.position),
                        ReflectValue::Vec3(after.position),
                    ),
                    (
                        "rotation",
                        ReflectValue::Quat(before.rotation),
                        ReflectValue::Quat(after.rotation),
                    ),
                    (
                        "scale",
                        ReflectValue::Vec3(before.scale),
                        ReflectValue::Vec3(after.scale),
                    ),
                ]
                .into_iter()
                .map(|(field, was, now)| Inverse::SetField {
                    entity: id,
                    component: component.clone(),
                    field: field.to_owned(),
                    before: was,
                    after: now,
                })
                .collect(),
            ))
        }
        EditorAction::AddComponent { entity, component } => Some(Inverse::RemoveComponent {
            entity: mirror.remote_of(*entity)?,
            component: component_name(resources, *component)?,
        }),
        EditorAction::RemoveComponent { entity, component } => {
            let name = component_name(resources, *component)?;
            Some(Inverse::AddComponent {
                entity: mirror.remote_of(*entity)?,
                state: entity_state::capture_component(resources, *entity, &name)?,
            })
        }
        EditorAction::Reparent { entity, new_parent } => {
            let id = mirror.remote_of(*entity)?;
            Some(Inverse::Reparent {
                entity: id,
                before: current_parent(mirror, resources, id),
                after: new_parent.and_then(|parent| mirror.remote_of(parent)),
            })
        }
        // The whole subtree, because that is what the project's despawn
        // takes with it.
        EditorAction::Despawn(entity) => {
            let id = mirror.remote_of(*entity)?;
            Some(Inverse::Recreate(subtrees(resources, mirror, &[id])))
        }
        _ => None,
    }
}

/// Files a sent edit in the history.
pub(crate) fn record_step(resources: &mut Resources, label: &str, inverse: Inverse) {
    if let Some(mut history) = resources.remove::<RemoteHistory>() {
        history.record(Step {
            label: label.to_owned(),
            inverse,
            key: None,
        });
        resources.insert(history);
    }
}

pub(crate) fn record(
    resources: &mut Resources,
    action: &EditorAction,
    before: Option<Inverse>,
    created: Vec<EntityId>,
) {
    // Loading a scene replaces the world every step in the history describes. Keeping them would
    // offer to undo an edit to an entity that no longer exists, against ids the project has since
    // reused.
    if matches!(
        action,
        EditorAction::OpenScene { .. } | EditorAction::CloseScene(_)
    ) {
        if let Some(history) = resources.get_mut::<RemoteHistory>() {
            history.clear();
        }
        return;
    }

    let inverse = match (before, created.is_empty()) {
        (Some(inverse), _) => inverse,
        (None, false) => Inverse::Despawn(created),
        (None, true) => return,
    };
    let label = label_of(action);
    let key = merge_key_of(action);
    let rides_along = rides_along(action, resources);
    if resources.get::<RemoteHistory>().is_none() {
        resources.insert(RemoteHistory::default());
    }
    if let Some(history) = resources.get_mut::<RemoteHistory>() {
        match rides_along {
            true => history.attach(inverse),
            false => history.record(Step {
                label,
                inverse,
                key,
            }),
        }
    }
}

/// What the Edit menu calls this step.
fn label_of(action: &EditorAction) -> String {
    match action {
        EditorAction::Spawn { .. } => "Spawn Entity".to_owned(),
        EditorAction::SpawnMesh { .. } => "Spawn Mesh Entity".to_owned(),
        EditorAction::SpawnBlock { .. } => "Spawn Block".to_owned(),
        EditorAction::Despawn(_) => "Despawn Entity".to_owned(),
        EditorAction::Duplicate(_) => "Duplicate Entity".to_owned(),
        EditorAction::PasteEntities { .. } => "Paste".to_owned(),
        EditorAction::MoveToScene { .. } => "Move to Scene".to_owned(),
        EditorAction::InstantiatePrefab { .. } => "Instantiate Prefab".to_owned(),
        EditorAction::SetField { field, .. } => format!("Set {field}"),
        EditorAction::AddComponent { .. } => "Add Component".to_owned(),
        EditorAction::RemoveComponent { .. } => "Remove Component".to_owned(),
        EditorAction::Reparent { .. } => "Reparent".to_owned(),
        EditorAction::TransformEdit { desc, .. } => (*desc).to_owned(),
        _ => "Edit".to_owned(),
    }
}

/// Whether this edit is the editor's bookkeeping rather than something
/// the user did.
///
/// 🔴 An override write is appended to the batch by
/// [`prefab_overrides::record`](super::prefab_overrides::record) — it
/// records that a field of a prefab instance no longer follows the
/// prefab. As a step of its own it costs a second Ctrl+Z for one action,
/// and it does something worse than that: it sits **between** two edits
/// to the same field, and the merge rule only ever looks at the top of
/// the stack. So on a prefab instance nothing ever merged, and a drag
/// went back to filing a step per frame.
///
/// Measured, not guessed. One drag and one typed value on an instance:
///
/// ```text
/// new step  label=Move Entity    depth=1
/// new step  label=Set overrides  depth=2
/// new step  label=Set position   depth=3
/// new step  label=Set overrides  depth=4
/// new step  label=Set position   depth=5   <- did not merge with depth=3
/// new step  label=Set overrides  depth=6
/// ```
fn rides_along(action: &EditorAction, resources: &Resources) -> bool {
    let EditorAction::SetField { component, .. } = action else {
        return false;
    };
    component_name(resources, *component)
        .and_then(|name| name.rsplit("::").next().map(str::to_owned))
        .is_some_and(|name| name == "PrefabInstance" || name == "PrefabMember")
}

/// What a run of edits to the same thing looks like.
fn merge_key_of(action: &EditorAction) -> Option<crate::history::MergeKey> {
    use crate::history::MergeKey;
    match action {
        EditorAction::SetField {
            entity,
            component,
            field,
            ..
        } => Some(MergeKey::of((
            entity.index(),
            entity.generation(),
            component,
            field,
        ))),
        EditorAction::TransformEdit { entity, .. } => Some(MergeKey::of((
            entity.index(),
            entity.generation(),
            "transform",
        ))),
        _ => None,
    }
}

/// The interned name behind a [`ComponentId`], which is how the project
/// keys components.
fn component_name(
    resources: &Resources,
    component: kooch_ecs::component::ComponentId,
) -> Option<String> {
    resources
        .get::<kooch_ecs::component::ComponentNames>()?
        .name(component)
        .map(str::to_owned)
}

mod inverse;

use super::remote_edit;
use inverse::*;

#[cfg(test)]
mod tests;
