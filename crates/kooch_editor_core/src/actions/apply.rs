//! Applying a frame's actions, and catching the prefab overwrites that need asking first.

use super::*;

#[derive(Default)]
pub(crate) struct DirtyPrefabs(std::collections::HashSet<kooch_core::Guid>);

impl DirtyPrefabs {
    pub(crate) fn contains(&self, prefab: kooch_core::Guid) -> bool {
        self.0.contains(&prefab)
    }

    pub(crate) fn mark(&mut self, prefab: kooch_core::Guid) {
        self.0.insert(prefab);
    }

    pub(crate) fn clear(&mut self, prefab: kooch_core::Guid) {
        self.0.remove(&prefab);
    }
}

/// A prefab save waiting on the user's answer about replacing a file.
#[derive(Clone)]
pub(crate) struct PendingPrefabOverwrite {
    pub(crate) entity: Entity,
    pub(crate) dest: Option<std::path::PathBuf>,
    /// The file that would be replaced. Shown to the user, so they are
    /// answering about a name they recognise rather than about "a prefab".
    pub(crate) path: std::path::PathBuf,
}

/// Holds back any `SavePrefab` that would replace an existing file.
pub(super) fn intercept_prefab_overwrites<'a>(
    resources: &mut Resources,
    actions: &'a [EditorAction],
) -> Vec<&'a EditorAction> {
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        // The answer arrived; the prompt has done its job either way.
        if matches!(
            action,
            EditorAction::CancelPrefabOverwrite
                | EditorAction::SavePrefab {
                    overwrite: true,
                    ..
                }
        ) {
            resources.remove::<PendingPrefabOverwrite>();
        }
        let EditorAction::SavePrefab {
            entity,
            dest,
            overwrite: false,
        } = action
        else {
            out.push(action);
            continue;
        };
        // No project open: the handler says so. Not this function's job to
        // report, and holding the action back would swallow the message.
        let Some(root) = crate::actions::handlers::prefab_root(resources) else {
            out.push(action);
            continue;
        };
        let name = crate::actions::handlers::entity_name(resources, *entity);
        let path = crate::actions::handlers::prefab_path(&root, &name, dest.as_deref());
        if !path.exists() {
            out.push(action);
            continue;
        }
        resources.insert(PendingPrefabOverwrite {
            entity: *entity,
            dest: dest.clone(),
            path,
        });
    }
    out
}

pub(crate) fn apply_actions(
    resources: &mut Resources,
    actions: &[EditorAction],
    undo_stack: &mut UndoStack,
) {
    // Dual-sink: with a connected remote session the editor's ECS is a mirror of a project that
    // owns the real state, so ECS edits route over the wire instead of mutating the mirror (which
    // the next refresh would overwrite).
    let mut queued: Vec<EditorAction> = resources
        .get_mut::<prefab_propagate::PendingPropagation>()
        .map(|pending| pending.drain())
        .unwrap_or_default()
        .into_iter()
        .map(EditorAction::PropagatePrefab)
        .collect();
    // Ahead of the propagation, so the project has dropped its stale copy
    // before anything asks it to rebuild from one.
    let reloads: Vec<EditorAction> = resources
        .get_mut::<handlers::PendingHostReloads>()
        .map(|pending| std::mem::take(&mut pending.0))
        .unwrap_or_default()
        .into_iter()
        .map(EditorAction::ReloadAssetOnHost)
        .collect();
    if !reloads.is_empty() {
        queued.splice(0..0, reloads);
    }
    // Ahead of everything: a world held across a rebuild has to be back
    // before anything else acts on the scene it is supposed to be in.
    let resumed = crate::carry::resume(resources);
    if !resumed.is_empty() {
        queued.splice(0..0, resumed);
    }
    if !queued.is_empty() {
        // 🔴 `debug`, not `info`. A live prefab drains every frame, so at `info` this printed sixty
        // identical lines a second and buried every other message in the Console — including the
        // ones a measurement run is there to read.
        tracing::debug!(
            target: "kooch_editor_core::prefab",
            drained = queued.len(),
            "propagation drained into actions",
        );
    }

    // Asked before the local/remote split so the prompt appears once
    // regardless of which path would have written the file.
    let mut actions = intercept_prefab_overwrites(resources, actions);
    actions.extend(queued.iter());
    let actions = &actions;

    // Recorded before the edits are applied, while the instance still holds the values the user is
    // changing away from — and appended so the write that persists the set travels the same path as
    // the edit that caused it.
    let recorded = prefab_overrides::record(resources, actions);
    let mut owned: Vec<&EditorAction>;
    let actions = match recorded.is_empty() {
        true => actions,
        false => {
            owned = actions.clone();
            owned.extend(recorded.iter());
            &owned
        }
    };

    let remote = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|s| s.is_connected());

    // A session that exists but has not answered yet: the project is still building and its world
    // has not arrived. Dropping the actions that need it is what stops a Ctrl+S from writing the
    // empty mirror over the project's scene — see `needs_a_live_world`.
    let awaiting_world = !remote
        && resources
            .get::<crate::remote_session::RemoteState>()
            .is_some_and(|state| state.session.is_some());
    if awaiting_world {
        let held = actions
            .iter()
            .filter(|action| action.needs_a_live_world())
            .count();
        if held > 0 {
            tracing::warn!(
                refused = held,
                "the project is still starting — edits are refused until its world arrives",
            );
        }
        for action in actions.iter().copied().filter(|a| !a.needs_a_live_world()) {
            apply_non_ecs_action(action, resources, undo_stack);
        }
        return;
    }

    // 🔴 A playing project owns its world and the editor does not get to touch it.
    let playing = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing);
    if playing {
        let refused = actions.iter().filter(|a| a.is_a_world_edit()).count();
        if refused > 0 {
            tracing::warn!(
                refused,
                "the project is playing — stop it to edit the world",
            );
        }
    }

    if remote {
        for action in actions.iter().copied() {
            if playing && action.is_a_world_edit() {
                continue;
            }
            if !remote_edit::dispatch(resources, action) {
                apply_non_ecs_action(action, resources, undo_stack);
            }
        }
        return;
    }

    let mut i = 0;
    while i < actions.len() {
        let action = actions[i];

        // Undo/Redo are handled directly — the scene's here, and every
        // other document by the handler below.
        if let EditorAction::Undo(document) | EditorAction::Redo(document) = action {
            let undo = matches!(action, EditorAction::Undo(_));
            match (document.is_world(), undo) {
                (true, true) => undo_stack.undo(resources),
                (true, false) => undo_stack.redo(resources),
                (false, _) => {
                    crate::history::documents::step(resources, document, undo);
                }
            }
            i += 1;
            continue;
        }

        // Check if this is an ECS action that can be batched.
        if action_to_command(action, resources).is_some() {
            // Find the run of consecutive same-variant ECS actions.
            let run_start = i;
            let mut run_end = i + 1;
            while run_end < actions.len() && same_ecs_variant(action, &actions[run_end]) {
                run_end += 1;
            }
            let run = &actions[run_start..run_end];

            if run.len() == 1 {
                // Single action — execute directly (snapshot already captured above
                // was discarded; re-capture since resources may have changed).
                if let Some(cmd) = action_to_command(run[0], resources) {
                    undo_stack.execute(cmd, resources);
                }
            } else {
                // Multiple same-type actions — batch into a CompoundCommand.
                let desc = batch_description(run);
                let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::with_capacity(run.len());
                for a in run.iter().copied() {
                    // Snapshot must be taken sequentially: each command's
                    // before-state depends on the previous command's execution.
                    if let Some(cmd) = action_to_command(a, resources) {
                        cmds.push(cmd);
                    }
                }
                let compound = CompoundCommand::new(desc, cmds);
                undo_stack.execute(Box::new(compound), resources);
            }

            i = run_end;
            continue;
        }

        // Non-ECS actions: process directly (no undo).
        apply_non_ecs_action(action, resources, undo_stack);
        i += 1;
    }
}
