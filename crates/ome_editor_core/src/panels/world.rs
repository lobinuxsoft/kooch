//! World panel — entity hierarchy list with context menu.

pub(crate) mod entity_row;
mod scene_bar;
mod spawn_menu;

use ome_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo, SceneDisplayInfo};

use self::entity_row::draw_entity_row;
use self::scene_bar::draw_scene_bar;
use self::spawn_menu::{draw_spawn_menu, spawn_entries};

/// Content of the "World" tab — entity hierarchy list with context menu.
pub(crate) fn draw_world_content(
    ui: &mut egui::Ui,
    focused: bool,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    entity_count: usize,
    archetype_count: usize,
    active_archetype_count: usize,
    last_clicked_index: &mut Option<usize>,
    scenes: &[SceneDisplayInfo],
) {
    draw_scene_bar(ui, scenes, actions);
    ui.label(format!(
        "{} entities, {} archetypes ({} active)",
        entity_count, archetype_count, active_archetype_count,
    ));
    ui.separator();

    ui.horizontal(|ui| {
        draw_spawn_menu(ui, actions);
        let any_selected = !selected.is_empty();
        if ui
            .add_enabled(
                any_selected,
                egui::Button::new(format!("{} Duplicate", icons::COPY)),
            )
            .on_hover_text(
                "Clone the selected entity (or entities) with every \
                 component value preserved. The new entity gets a fresh \
                 handle; nothing about the source is touched.",
            )
            .clicked()
        {
            for &entity in selected.iter() {
                actions.push(EditorAction::Duplicate(entity));
            }
        }
        if ui
            .add_enabled(
                any_selected,
                egui::Button::new(format!("{} Despawn", icons::TRASH)),
            )
            .clicked()
        {
            for entity in selected.drain(..) {
                actions.push(EditorAction::Despawn(entity));
            }
        }
    });
    ui.separator();

    handle_keyboard(ui, focused, entities, selected, last_clicked_index, actions);

    egui::ScrollArea::vertical()
        .id_salt("world_tree")
        .show(ui, |ui| {
            // One scene: no headers, since every row would sit under the same
            // one and the grouping would only cost a level of indentation.
            if scenes.len() < 2 {
                for (idx, info) in entities.iter().enumerate() {
                    draw_entity_row(
                        ui,
                        idx,
                        info,
                        entities,
                        selected,
                        reflected_types,
                        actions,
                        last_clicked_index,
                    );
                }
            } else {
                draw_grouped_by_scene(
                    ui,
                    entities,
                    scenes,
                    selected,
                    reflected_types,
                    actions,
                    last_clicked_index,
                );
            }

            // Empty space: click to deselect, right-click to create, drop
            // target to unparent an entity.
            let remaining = ui.available_rect_before_wrap();
            let empty_resp = ui.allocate_rect(remaining, egui::Sense::click_and_drag());
            if empty_resp.clicked() {
                selected.clear();
                *last_clicked_index = None;
            }

            // The same entries the toolbar's Spawn button offers, reached
            // where people actually reach for them: right-click in the
            // empty part of the hierarchy. A row's own right-click menu
            // handles per-entity actions, including Add Component (#591).
            empty_resp.context_menu(|ui| {
                spawn_entries(ui, actions);
            });
            // A prefab dropped into the hierarchy spawns at the position it
            // was authored at: a list of names has no geometry to read a
            // place out of, and defaulting to the origin would silently move
            // a prefab that was deliberately authored elsewhere. Drop it in
            // the View panel to choose a spot.
            if empty_resp
                .dnd_hover_payload::<crate::drag_drop::DraggedPrefab>()
                .is_some()
            {
                ui.painter().rect_filled(
                    remaining,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
                );
                if let Some(prefab) =
                    empty_resp.dnd_release_payload::<crate::drag_drop::DraggedPrefab>()
                {
                    actions.push(EditorAction::InstantiatePrefab {
                        path: prefab.path.clone(),
                        at: crate::viewport_pick::DropPoint::Authored,
                    });
                }
            }
            if empty_resp.dnd_hover_payload::<Entity>().is_some() {
                ui.painter().rect_filled(
                    remaining,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(100, 100, 100, 20),
                );
            }
            if let Some(dragged) = empty_resp.dnd_release_payload::<Entity>() {
                let d = *dragged;
                if entities.iter().any(|e| e.entity == d && e.parent.is_some()) {
                    actions.push(EditorAction::Reparent {
                        entity: d,
                        new_parent: None,
                    });
                }
            }
        });
}

/// Keyboard shortcuts for the World panel: Delete, Ctrl+A, arrow up/down.
fn handle_keyboard(
    ui: &egui::Ui,
    focused: bool,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    last_clicked_index: &mut Option<usize>,
    actions: &mut Vec<EditorAction>,
) {
    // Navigation belongs to the panel with focus. Without this the arrows
    // moved the hierarchy's selection from inside the Console, and Ctrl+A
    // fought the select-all of whatever text field was being typed in
    // (#661).
    //
    // Document shortcuts stay global on purpose — Ctrl+Z, Ctrl+S, Play are
    // about the project, not about a panel — and they live elsewhere.
    if !focused {
        return;
    }

    // Delete/Suprimir: despawn selected entities.
    let kb_delete = ui.input(|i| i.key_pressed(egui::Key::Delete));
    if kb_delete && !selected.is_empty() {
        for entity in selected.drain(..) {
            actions.push(EditorAction::Despawn(entity));
        }
        *last_clicked_index = None;
    }

    // Keyboard navigation: Ctrl+A to select all.
    let kb_select_all = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A));
    if kb_select_all && !entities.is_empty() {
        selected.clear();
        selected.extend(entities.iter().map(|e| e.entity));
        *last_clicked_index = Some(entities.len() - 1);
    }

    // Keyboard navigation: Arrow Up/Down.
    let kb_up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
    let kb_down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
    let kb_shift = ui.input(|i| i.modifiers.shift);

    if (kb_up || kb_down) && !entities.is_empty() {
        let current_idx = last_clicked_index.unwrap_or(0);
        let new_idx = if kb_up {
            current_idx.saturating_sub(1)
        } else {
            (current_idx + 1).min(entities.len() - 1)
        };

        if kb_shift {
            // Extend selection to include the new index.
            let entity = entities[new_idx].entity;
            if !selected.contains(&entity) {
                selected.push(entity);
            }
        } else {
            // Move selection to the new index.
            selected.clear();
            selected.push(entities[new_idx].entity);
        }
        *last_clicked_index = Some(new_idx);
    }
}

/// Draws the entity list grouped under a header per open scene.
///
/// Only used with more than one scene open. Rows keep their original
/// indices so keyboard navigation and shift-range selection still address
/// the same flat list the rest of the panel works with.
#[allow(clippy::too_many_arguments)]
fn draw_grouped_by_scene(
    ui: &mut egui::Ui,
    entities: &[EntityDisplayInfo],
    scenes: &[SceneDisplayInfo],
    selected: &mut Vec<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    last_clicked_index: &mut Option<usize>,
) {
    let mut drawn = vec![false; entities.len()];

    for scene in scenes {
        let rows: Vec<usize> = entities
            .iter()
            .enumerate()
            .filter(|(_, info)| info.scene == Some(scene.id))
            .map(|(idx, _)| idx)
            .collect();

        let header = if scene.dirty {
            format!("{} ({} entities) *", scene.name, rows.len())
        } else {
            format!("{} ({} entities)", scene.name, rows.len())
        };

        // The active scene starts expanded: it is the one being worked in.
        egui::CollapsingHeader::new(header)
            .id_salt(scene.id.to_string())
            .default_open(scene.active)
            .show(ui, |ui| {
                if rows.is_empty() {
                    ui.weak("(empty)");
                }
                for idx in rows {
                    drawn[idx] = true;
                    draw_entity_row(
                        ui,
                        idx,
                        &entities[idx],
                        entities,
                        selected,
                        reflected_types,
                        actions,
                        last_clicked_index,
                    );
                }
            });
    }

    // Anything belonging to no scene still has to be reachable, or an
    // entity spawned before the first save would vanish from the panel
    // that is supposed to list the world.
    let orphans: Vec<usize> = drawn
        .iter()
        .enumerate()
        .filter(|&(_, done)| !done)
        .map(|(idx, _)| idx)
        .collect();
    if !orphans.is_empty() {
        egui::CollapsingHeader::new(format!("Unsaved ({} entities)", orphans.len()))
            .id_salt("world_unsaved_group")
            .default_open(true)
            .show(ui, |ui| {
                ui.weak("Not in any scene yet — saved with the active one.");
                for idx in orphans {
                    draw_entity_row(
                        ui,
                        idx,
                        &entities[idx],
                        entities,
                        selected,
                        reflected_types,
                        actions,
                        last_clicked_index,
                    );
                }
            });
    }
}
