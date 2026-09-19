//! What the render system asks along the way: the launched project's output, viewport clicks, histories and preview images.

use super::*;

/// Moves the mirrored project's stdout into the editor's log.
pub(super) fn forward_remote_output(resources: &mut Resources) {
    let Some(state) = resources.get::<crate::remote_session::RemoteState>() else {
        return;
    };
    let Some(session) = state.session.as_ref() else {
        return;
    };
    // Kept as well as forwarded, while the handshake is still in flight: the log is where these
    // belong, but the connecting banner needs something to show and draining is destructive (#672).
    // Once the project answers, the Console is the place to read it and the copy stops growing.
    let keep = session.state() == crate::remote_session::ConnectionState::Connecting;
    let Some(buffer) = resources.get::<kooch_core::LogBuffer>() else {
        return;
    };
    let buffer = buffer.clone();
    let lines = session.drain_output();
    for line in &lines {
        crate::project_log::record(&buffer, line);
    }
    if keep
        && !lines.is_empty()
        && let Some(state) = resources.get_mut::<crate::remote_session::RemoteState>()
    {
        state.connect_output.extend(lines);
    }
}

/// Selects the entity under the cursor, if the viewport was clicked.
pub(super) fn apply_viewport_click(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    overlay: &mut EditorOverlay,
) {
    if !delta.lmb_clicked {
        return;
    }
    let playing = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing);
    if playing {
        return;
    }
    let Some(cursor) = delta.cursor_local else {
        return;
    };

    // Element mode first. A miss falls through to entity picking only when it lands on another
    // entity, so clicking past the edited block onto empty space keeps it in the inspector.
    if overlay.element_mode.edits_elements()
        && let [entity] = overlay.selected_entities.as_slice()
    {
        let entity = *entity;
        let element = crate::block_edit::element_under(
            resources,
            entity,
            cursor,
            delta.viewport_size,
            overlay.element_mode,
        );
        let hit = crate::picking::entity_hit_at(resources, cursor, delta.viewport_size);
        let block = match element {
            Some(_) => {
                crate::block_edit::block_distance(resources, entity, cursor, delta.viewport_size)
            }
            None => None,
        };
        if let crate::block_edit::ElementClick::Switch(other) =
            crate::block_edit::resolve_click(entity, element, hit, block)
        {
            if let Some(selection) = resources.get_mut::<crate::block_edit::BlockSelection>() {
                selection.clear();
            }
            overlay.selected_entities.clear();
            overlay.selected_entities.push(other);
            return;
        }
        if let Some(mut selection) = resources.remove::<crate::block_edit::BlockSelection>() {
            crate::block_edit::apply_click(&mut selection, entity, element, delta.ctrl_held);
            resources.insert(selection);
        }
        return;
    }

    let hit = crate::picking::entity_at(resources, cursor, delta.viewport_size);
    match (hit, delta.ctrl_held) {
        // Ctrl adds and removes, the same chord the World panel uses, so
        // building a multi-selection does not depend on which panel it was
        // started in.
        (Some(entity), true) => match overlay.selected_entities.iter().position(|e| *e == entity) {
            Some(index) => {
                overlay.selected_entities.remove(index);
            }
            None => overlay.selected_entities.push(entity),
        },
        (Some(entity), false) => {
            overlay.selected_entities.clear();
            overlay.selected_entities.push(entity);
        }
        (None, false) => overlay.selected_entities.clear(),
        // Ctrl+click on nothing is a miss, not "deselect everything".
        (None, true) => {}
    }
}

/// What the scene's history can offer, from whichever one is driving it.
pub(super) fn world_history(
    resources: &kooch_core::resource::Resources,
    undo_stack: &UndoStack,
) -> (bool, bool, Option<String>, Option<String>) {
    let remote = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.is_connected())
        .then(|| resources.get::<crate::actions::remote_undo::RemoteHistory>())
        .flatten();
    match remote {
        Some(history) => (
            history.can_undo(),
            history.can_redo(),
            history.undo_description().map(String::from),
            history.redo_description().map(String::from),
        ),
        None => (
            undo_stack.can_undo(),
            undo_stack.can_redo(),
            undo_stack.undo_description().map(String::from),
            undo_stack.redo_description().map(String::from),
        ),
    }
}

/// Whether a guid names a prefab or an ordinary asset.
pub(super) fn asset_kind(
    resources: &kooch_core::resource::Resources,
    guid: kooch_core::Guid,
) -> crate::history::AssetKind {
    let prefab = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|db| db.entry(guid)?.type_name.clone())
        .is_some_and(|name| name == std::any::type_name::<kooch_ecs::scene::SceneDocument>());
    match prefab {
        true => crate::history::AssetKind::Prefab,
        false => crate::history::AssetKind::Asset,
    }
}

/// Files the open graph's previous state when the panel changed it this frame.
pub(super) fn record_graph_edit(resources: &mut Resources, edited: &crate::state::OpenShaderGraph) {
    let Some(step) = resources
        .get::<crate::state::OpenShaderGraph>()
        .filter(|before| before.path == edited.path)
        .and_then(|before| {
            crate::shader_graph::change(&before.graph, &edited.graph).or_else(|| {
                (before.annotations != edited.annotations)
                    .then_some(crate::shader_graph::GraphStep::Annotate)
            })
        })
    else {
        return;
    };
    crate::history::documents::record(
        resources,
        &crate::history::Document::ShaderGraph(edited.path.clone()),
        step.label(),
        step.merge_key(&edited.path),
    );
}

/// Closes the current run of edits in every history.
pub(super) fn seal_histories(resources: &mut Resources) {
    if let Some(history) = resources.get_mut::<crate::actions::remote_undo::RemoteHistory>() {
        history.seal();
    }
    if let Some(histories) = resources.get_mut::<crate::history::documents::DocumentHistories>() {
        histories.seal();
    }
}

/// What F should frame for this entity.
pub(super) fn focus_target(
    resources: &mut Resources,
    entity: kooch_ecs::entity::Entity,
) -> Option<crate::editor_camera::framing::FocusTarget> {
    use crate::editor_camera::framing::{FocusTarget, radius_around};

    if let Some((min, max)) = crate::block_edit::selection_bounds(resources, entity) {
        let point = (min + max) * 0.5;
        return Some(FocusTarget {
            point,
            radius: Some(radius_around(point, min, max)),
        });
    }

    let point = entity_world_position(resources, entity)?;
    let radius = crate::picking::entity_bounds(resources, entity)
        .map(|(min, max)| radius_around(point, min, max));
    Some(FocusTarget { point, radius })
}

/// The image each texture node of the open graph is previewed with, by the parameter's name.
pub(super) fn preview_images(resources: &Resources) -> Vec<(String, kooch_core::Guid)> {
    resources
        .get::<crate::state::OpenShaderGraph>()
        .map(|open| {
            open.graph
                .nodes()
                .filter_map(|node| match node {
                    crate::shader_graph::Node::Texture {
                        name,
                        preview: Some(guid),
                        ..
                    } => Some((name.clone(), *guid)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Loads an image asset through the asset server, as the Inspector's asset detail does.
pub(super) fn loaded_image(
    resources: &mut Resources,
    guid: kooch_core::Guid,
) -> Option<kooch_render::texture::Image> {
    use kooch_core::asset_loader::AssetServer;
    use kooch_render::texture::Image;

    let mut server = resources.remove::<AssetServer>()?;
    let handle = server.load_by_guid::<Image>(guid, resources).ok();
    resources.insert(server);
    resources
        .get::<kooch_core::assets::Assets<Image>>()?
        .get(handle?)
        .cloned()
}
