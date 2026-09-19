//! The documents the editor opens in a panel of its own: shader graphs and input maps.

use super::*;

/// Reads an `.inputmap` and hands it to the panel.
/// Reads a generated `.shader` back into the Shader Graph panel (#1159).
pub(super) fn open_shader_graph(resources: &mut Resources, path: &std::path::Path) {
    let Ok(source) = std::fs::read_to_string(path) else {
        tracing::error!(file = %path.display(), "could not read the shader");
        return;
    };
    let Some(graph) = crate::shader_graph::extract(&source) else {
        // Written by hand, so there is nothing to draw: its text is the only view of it.
        tracing::info!(
            file = %path.display(),
            "this shader was written by hand; opening it in the IDE instead",
        );
        open_in_ide(resources, path);
        return;
    };
    resources.insert(crate::state::OpenShaderGraph {
        path: path.to_path_buf(),
        graph,
        annotations: crate::shader_graph::annotations::extract(&source),
        focus_requested: true,
        dirty: false,
    });
}

/// Generates the `.shader` from the open graph and writes it, so the render picks it up.
pub(super) fn save_shader_graph(resources: &mut Resources) {
    let Some(open) = resources.get::<crate::state::OpenShaderGraph>() else {
        return;
    };
    let (path, graph) = (open.path.clone(), open.graph.clone());
    let annotations = open.annotations.clone();
    let source = match crate::shader_graph::generate(&graph)
        .and_then(|source| crate::shader_graph::annotations::embed(source, &annotations))
    {
        Ok(source) => source,
        Err(reason) => {
            tracing::error!("the graph does not generate a shader: {reason}");
            return;
        }
    };
    if let Err(error) = std::fs::write(&path, source) {
        tracing::error!(file = %path.display(), %error, "could not write the shader");
        return;
    }
    crate::actions::handlers::asset_saved(resources, &path);
    if let Some(open) = resources.get_mut::<crate::state::OpenShaderGraph>() {
        open.dirty = false;
    }
    tracing::info!(file = %path.display(), "shader written from its graph");
}

pub(super) fn open_input_map(resources: &mut Resources, path: &std::path::Path) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            tracing::error!(file = %path.display(), error = %e, "could not read input map");
            return;
        }
    };
    // A standalone action opens as a map of one, so the panel needs no
    // second code path for it.
    let single = path
        .extension()
        .is_some_and(|e| e == kooch_input::actions::INPUT_ACTION_EXTENSION);
    let parsed = if single {
        ron::from_str::<kooch_input::actions::Action>(&text).map(|mut action| {
            action.ensure_id("");
            let name = action.name.clone();
            kooch_input::actions::ActionMap::new(name).add(action)
        })
    } else {
        ron::from_str::<kooch_input::actions::ActionMap>(&text).map(|mut map| {
            map.assign_missing_ids();
            map
        })
    };

    match parsed {
        Ok(map) => {
            tracing::info!(
                file = %path.display(),
                actions = map.actions.len(),
                "input map opened",
            );
            resources.insert(crate::state::OpenInputMap {
                path: path.to_path_buf(),
                kind: match single {
                    true => crate::state::OpenInputKind::SingleAction,
                    false => crate::state::OpenInputKind::Map,
                },
                map,
                focus_requested: true,
                selected: None,
                dirty: false,
            });
        }
        // Named rather than swallowed: a map that will not parse is a
        // file someone has to fix, and an empty panel says nothing about
        // which file or why.
        Err(e) => tracing::error!(
            file = %path.display(),
            error = %e,
            "input map could not be parsed",
        ),
    }
}

/// The processor list a target points at, if it points at one.
pub(super) fn processors_of(
    map: &mut kooch_input::actions::ActionMap,
    to: crate::panels::input_map::ProcessorTarget,
) -> Option<&mut Vec<kooch_input::actions::Processor>> {
    use crate::panels::input_map::ProcessorTarget;
    match to {
        ProcessorTarget::Action(index) => map.actions.get_mut(index).map(|a| &mut a.processors),
        ProcessorTarget::Binding(at) => map
            .actions
            .get_mut(at.action)
            .and_then(|action| action.bindings.get_mut(at.binding))
            .map(|binding| &mut binding.processors),
    }
}

/// Applies one edit to the open map, in memory.

/// What an input-map edit is called in the history, and whether it continues the one before it.
pub(super) fn undoable_step(
    edit: &crate::panels::input_map::InputMapAction,
) -> Option<(&'static str, Option<crate::history::MergeKey>)> {
    use crate::history::MergeKey;
    use crate::panels::input_map::InputMapAction as Edit;

    match edit {
        Edit::Save | Edit::Select(_) => None,
        // Typed and dragged: these arrive per keystroke and per frame, so
        // they carry a key and collapse into one step.
        Edit::RenameAction { action, .. } => {
            Some(("Rename Action", Some(MergeKey::of(("rename", action)))))
        }
        Edit::SetProcessor { to, index, .. } => Some((
            "Edit Processor",
            Some(MergeKey::of((format!("{to:?}"), index))),
        )),
        Edit::SetComposite { at, .. } => {
            Some(("Edit Composite", Some(MergeKey::of(format!("{at:?}")))))
        }
        // Discrete: one click, one step.
        Edit::SetControlType { .. } => Some(("Set Control Type", None)),
        Edit::Rebind { .. } => Some(("Rebind", None)),
        Edit::RemoveBinding(_) => Some(("Remove Binding", None)),
        Edit::AddBinding { .. } => Some(("Add Binding", None)),
        Edit::AddComposite { .. } => Some(("Add Composite", None)),
        Edit::AddProcessor { .. } => Some(("Add Processor", None)),
        Edit::RemoveProcessor { .. } => Some(("Remove Processor", None)),
        Edit::MoveProcessor { .. } => Some(("Move Processor", None)),
        _ => Some(("Edit Input Map", None)),
    }
}

pub(super) fn edit_input_map(
    resources: &mut Resources,
    edit: &crate::panels::input_map::InputMapAction,
) {
    use crate::panels::input_map::InputMapAction as Edit;
    use kooch_input::actions::{Action, Binding, ControlPath, ControlType, Role};
    use kooch_input::ids::KeyCode;

    // Before the edit lands, while the map still holds what an undo
    // wants. Reading the path first because `record` needs the document
    // and the borrow below is exclusive.
    let path = resources
        .get::<crate::state::OpenInputMap>()
        .map(|open| open.path.clone());
    if let Some(path) = path
        && let Some((label, key)) = undoable_step(edit)
    {
        crate::history::documents::record(
            resources,
            &crate::history::Document::InputMap(path),
            label,
            key,
        );
    }

    let Some(open) = resources.get_mut::<crate::state::OpenInputMap>() else {
        return;
    };
    let map = &mut open.map;
    let changed = match edit {
        Edit::Select(selection) => {
            open.selected = Some(*selection);
            false
        }
        Edit::RenameAction { action, name } => match map.actions.get_mut(*action) {
            Some(target) if target.name != *name => {
                target.name = name.clone();
                true
            }
            _ => false,
        },
        Edit::SetControlType {
            action,
            control_type,
        } => match map.actions.get_mut(*action) {
            Some(target) if target.control_type != *control_type => {
                target.control_type = *control_type;
                true
            }
            _ => false,
        },
        Edit::AddAction => {
            // Named for what it is until renamed. An empty name would
            // resolve to nothing and read as a broken action.
            let name = unique_action_name(map, "new_action");
            map.actions.push(Action::new(name, ControlType::Button));
            // Selected on the way in, so the properties pane is already
            // showing its name field. Otherwise a new action is a row
            // somewhere in a list with no hint that it wants a name.
            open.selected = Some(crate::panels::input_map::Selection::Action(
                map.actions.len() - 1,
            ));
            true
        }
        Edit::RemoveAction { action } => {
            if *action < map.actions.len() {
                map.actions.remove(*action);
                true
            } else {
                false
            }
        }
        Edit::AddBinding { action } => match map.actions.get_mut(*action) {
            // Unbound rather than guessing: `Space` on every new binding
            // would silently collide with whatever already uses it.
            Some(target) => {
                target
                    .bindings
                    .push(Binding::to(ControlPath::Key(KeyCode::Space)));
                true
            }
            None => false,
        },
        // The head plus one unbound part per name it declares. Adding a
        // bare head would put a composite in the list that reads as
        // nothing and gives no clue which parts are missing.
        Edit::AddComposite { action, composite } => match map.actions.get_mut(*action) {
            Some(target) => {
                use kooch_input::actions::PartName;
                target.bindings.push(Binding::composite(*composite));
                let head = target.bindings.len() - 1;
                for name in PartName::of(*composite) {
                    target
                        .bindings
                        .push(Binding::part(*name, ControlPath::Key(KeyCode::Space)));
                }
                // Selected so its Mode lands in the properties pane —
                // the setting that decides whether a diagonal outruns a
                // straight line, and the one nobody goes looking for.
                open.selected = Some(crate::panels::input_map::Selection::Binding(
                    crate::panels::input_map::BindingAddress {
                        action: *action,
                        binding: head,
                    },
                ));
                true
            }
            None => false,
        },
        Edit::SetComposite { at, composite } => match map
            .actions
            .get_mut(at.action)
            .and_then(|target| target.bindings.get_mut(at.binding))
        {
            // Only a head has parameters; aimed at a part this is a
            // no-op rather than a binding turned into something else.
            Some(binding) => match &mut binding.role {
                Role::CompositeHead(current) if current != composite => {
                    *current = *composite;
                    true
                }
                _ => false,
            },
            None => false,
        },
        Edit::AddProcessor { to, processor } => match processors_of(map, *to) {
            // Appended, so a new one runs last. Prepending would silently
            // change what every existing processor sees.
            Some(list) => {
                list.push(*processor);
                true
            }
            None => false,
        },
        Edit::SetProcessor {
            to,
            index,
            processor,
        } => match processors_of(map, *to).and_then(|list| list.get_mut(*index)) {
            Some(current) if current != processor => {
                *current = *processor;
                true
            }
            _ => false,
        },
        Edit::RemoveProcessor { to, index } => match processors_of(map, *to) {
            Some(list) if *index < list.len() => {
                list.remove(*index);
                true
            }
            _ => false,
        },
        Edit::MoveProcessor { to, index, delta } => match processors_of(map, *to) {
            Some(list) => {
                let target = *index as i32 + delta;
                // Refused rather than saturated: a move off the end that
                // silently stayed put would still mark the file unsaved.
                if target < 0 || target as usize >= list.len() {
                    false
                } else {
                    list.swap(*index, target as usize);
                    true
                }
            }
            None => false,
        },
        // A composite head takes its parts with it. Left behind they are
        // rows `groups` skips — saved to the file, read by nothing.
        Edit::RemoveBinding(at) => match map.actions.get_mut(at.action) {
            Some(target) if at.binding < target.bindings.len() => {
                let range = kooch_input::actions::group_range(&target.bindings, at.binding);
                target.bindings.drain(range);
                true
            }
            _ => false,
        },
        Edit::Rebind { at, path } => match map
            .actions
            .get_mut(at.action)
            .and_then(|a| a.bindings.get_mut(at.binding))
        {
            Some(binding) => {
                // Only what it reads changes. Processors and the part it
                // plays in a composite are properties of the binding, not
                // of the control behind it.
                binding.role = match &binding.role {
                    Role::Part { name, .. } => Role::Part {
                        name: *name,
                        path: *path,
                    },
                    _ => Role::Whole(*path),
                };
                true
            }
            None => false,
        },
        // Rebind prompts are panel state, not document edits, and Save
        // is routed before it gets here.
        Edit::Save => false,
    };

    if changed {
        open.dirty = true;
    }
}

/// A name no other action in `map` already has.
pub(super) fn unique_action_name(map: &kooch_input::actions::ActionMap, base: &str) -> String {
    if map.resolve(base).is_none() {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| map.resolve(name).is_none())
        .unwrap_or_else(|| base.to_owned())
}

/// Writes the open map back to its file.
pub(super) fn save_input_map(resources: &mut Resources) {
    let Some((path, map, kind)) = resources
        .get::<crate::state::OpenInputMap>()
        .map(|open| (open.path.clone(), open.map.clone(), open.kind))
    else {
        return;
    };
    // A standalone action is unwrapped from the map of one it was opened into, so the file keeps
    // the shape it had. Through `save_action` rather than a bare write: a file that reached disk
    // without its `.meta` is one nothing can reference.
    let written = match map.actions.first() {
        Some(action) => kooch_input::actions::save_action(action, &path),
        None => Err("the action was deleted; nothing to save".to_owned()),
    };
    let _ = kind;
    match written {
        Ok(guid) => {
            tracing::info!(file = %path.display(), %guid, "input map saved");
            // The panel edits a copy; without this the bindings on disk and
            // the ones the running project answers to stay different until
            // it is relaunched.
            crate::actions::handlers::asset_saved(resources, &path);
            if let Some(open) = resources.get_mut::<crate::state::OpenInputMap>() {
                open.dirty = false;
            }
        }
        // Left dirty on purpose: the edits are still the only copy that
        // has them, and clearing the flag would claim they are safe.
        Err(e) => tracing::error!(file = %path.display(), error = %e, "failed to save input map"),
    }
}
