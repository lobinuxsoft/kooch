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
///
/// Deliberately not "the action, reversed": several actions share an
/// inverse (adding a component and pasting an entity are both undone by
/// something already in this list), and a few have no reversed form at
/// all — undoing a despawn is a *creation*, and it produces different
/// entity ids than the ones that went away.
pub(crate) enum Inverse {
    /// Both sides of a field edit.
    ///
    /// 🔴 Carrying `after` too, rather than reading the current value
    /// when the undo runs. The mirror is a **snapshot on a refresh
    /// timer**: between the edit and the Ctrl+Z it can be anything from
    /// current to half a second stale, and a redo built from a stale read
    /// would put back a value that was never there. Both values are known
    /// at record time and neither changes afterwards.
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
    /// Several edits that have to travel together, applied in order.
    ///
    /// A gizmo drag is one action and one entry; a multi-selection
    /// despawn is several actions the user thinks of as one.
    Several(Vec<Inverse>),
}

/// One entity to bring back, and where it hung.
pub(crate) struct Reborn {
    pub state: EntityState,
    pub parent: Option<Ancestor>,
}

/// A parent that either still exists or is being recreated alongside.
///
/// A subtree recreates its own links: the child's parent is not an id yet
/// when the batch is built, it is *the third entry in this batch*, and it
/// becomes an id when that entry is spawned.
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
    ///
    /// Clears the redo stack, like every undo history: the branch those
    /// steps undid no longer exists.
    pub fn record(&mut self, step: Step) {
        let sealed = std::mem::take(&mut self.sealed);
        // A continuation keeps the *older* step's before-state — that is
        // what an undo has to reach — and takes the newer one's
        // after-state, which is what a redo has to write. Sixty frames of
        // a drag become one step holding where it started and where it
        // ended.
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
        // 🔴 One line per step the history takes, because "how many steps
        // did that edit file?" is not answerable from the outside: the
        // symptom of getting it wrong is a Ctrl+Z that needs pressing
        // twice, and by then the evidence is gone.
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
    ///
    /// The step keeps its label and its merge key, so the edit that
    /// follows still continues the run — which is the half of this that
    /// makes coalescing work on a prefab instance at all.
    ///
    /// With nothing above it to belong to, it becomes a step of its own
    /// rather than being dropped: an override write that reached the
    /// project has to be undoable by something.
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
///
/// Returns `true` when a step was taken. `false` means the stack was
/// empty or the session had gone — the caller has nothing else to try
/// either way, since the local stack describes a different world.
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
        // 🔴 The step is *not* put back. It described a world that no
        // longer matches — an entity someone else deleted, a component
        // the project refused — and a history whose next step is known to
        // fail is worse than a short one, because the second Ctrl+Z would
        // hit it again.
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
///
/// `None` covers three different things and they are all the same to the
/// caller: an edit that creates (whose inverse is only knowable *after*
/// the send, from the ids that came back), an edit that is not undoable
/// (saving a scene, pressing Play), and an edit whose before-state could
/// not be read — a component the mirror has no value for.
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
///
/// Called after the send, so a creation can be undone by despawning what
/// it actually created rather than what it was asked to create.
pub(crate) fn record(
    resources: &mut Resources,
    action: &EditorAction,
    before: Option<Inverse>,
    created: Vec<EntityId>,
) {
    // Loading a scene replaces the world every step in the history
    // describes. Keeping them would offer to undo an edit to an entity
    // that no longer exists, against ids the project has since reused.
    if matches!(action, EditorAction::OpenScene) {
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
///
/// The same words the local commands use, because the menu shows one of
/// the two and the user is not supposed to be able to tell which.
fn label_of(action: &EditorAction) -> String {
    match action {
        EditorAction::Spawn { .. } => "Spawn Entity".to_owned(),
        EditorAction::SpawnMesh { .. } => "Spawn Mesh Entity".to_owned(),
        EditorAction::Despawn(_) => "Despawn Entity".to_owned(),
        EditorAction::Duplicate(_) => "Duplicate Entity".to_owned(),
        EditorAction::PasteEntities => "Paste".to_owned(),
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
///
/// 🔴 Only the two that arrive continuously. The Inspector emits an edit
/// per `changed()` — one per keystroke, one per frame of a drag — and a
/// gizmo emits one per drag. Everything else here is a click, and two
/// clicks are two steps however fast they were.
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

impl Inverse {
    /// Folds a later edit into this one, keeping this one's before-state.
    ///
    /// Only the paired kinds can absorb: a field edit knows both sides,
    /// so the merged step is "from where it started to where it ended".
    /// Anything else keeps what it has — a step that cannot merge should
    /// never have carried a key in the first place.
    fn absorb(&mut self, newer: Inverse) {
        match (self, newer) {
            (
                Inverse::SetField { after, .. },
                Inverse::SetField {
                    after: newer_after, ..
                },
            ) => *after = newer_after,
            // Zip stops at the shorter of the two, which is what keeps a
            // transform's three fields merging after bookkeeping has been
            // appended as a fourth: the rider keeps the oldest state,
            // which is the one an undo wants.
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
    ///
    /// Appended rather than prepended: the edit stays first, so a later
    /// edit to the same field merges into it and the rider is left
    /// holding the state it started with.
    fn attach(&mut self, rider: Inverse) {
        match self {
            Inverse::Several(mine) => mine.push(rider),
            other => {
                let edit = std::mem::replace(other, Inverse::Despawn(Vec::new()));
                *other = Inverse::Several(vec![edit, rider]);
            }
        }
    }

    /// Sends this inverse, and returns the one that reverses *it*.
    ///
    /// Every arm captures before it sends: what an undo needs to know is
    /// the state that is about to be replaced, and after the send it is
    /// gone.
    pub(crate) fn apply(
        self,
        client: &RemoteClient,
        mirror: &RemoteMirror,
        resources: &Resources,
    ) -> Result<Inverse, String> {
        match self {
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
///
/// In order, and that order matters: [`Ancestor::Batch`] names an entry
/// by index, so a child may only refer to an entry already spawned. The
/// captures in [`subtrees`] are built parent-first for exactly this.
///
/// The entity itself is built by [`super::remote_edit::build`] — the same
/// call duplicate and paste use, so an entity that comes back from an
/// undo is assembled exactly like one that was just created.
fn rebuild(
    client: &RemoteClient,
    mirror: &RemoteMirror,
    reborn: &[Reborn],
) -> Result<Vec<EntityId>, String> {
    let mut created: Vec<EntityId> = Vec::with_capacity(reborn.len());
    for entry in reborn {
        let id = super::remote_edit::build(client, mirror, &entry.state)?;
        created.push(id);
        let parent = match entry.parent {
            Some(Ancestor::Existing(id)) => Some(id),
            Some(Ancestor::Batch(index)) => created.get(index).copied(),
            None => None,
        };
        if parent.is_some()
            && let Err(e) = client.set_parent(id, parent)
        {
            tracing::warn!(
                target: "kooch_editor_core::remote_undo",
                "the entity came back unparented: {e}",
            );
        }
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

fn capture_into(
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
fn children_of(
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
fn current_parent(
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
fn field_value(
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

fn local_of(mirror: &RemoteMirror, entity: EntityId) -> Option<kooch_ecs::entity::Entity> {
    mirror.local_of(entity)
}

#[cfg(test)]
mod tests;
