//! World panel — entity hierarchy list with context menu.

mod entity_row;
mod scene_bar;
mod spawn_menu;

use ome_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo, SceneDisplayInfo};

use self::entity_row::draw_entity_row;
use self::scene_bar::draw_scene_bar;
use self::spawn_menu::draw_spawn_menu;

/// Content of the "World" tab — entity hierarchy list with context menu.
pub(crate) fn draw_world_content(
    ui: &mut egui::Ui,
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

    handle_keyboard(ui, entities, selected, last_clicked_index, actions);

    egui::ScrollArea::vertical().show(ui, |ui| {
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

        // Empty space: click to deselect, drop target to unparent an entity.
        let remaining = ui.available_rect_before_wrap();
        let empty_resp = ui.allocate_rect(remaining, egui::Sense::click_and_drag());
        if empty_resp.clicked() {
            selected.clear();
            *last_clicked_index = None;
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
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    last_clicked_index: &mut Option<usize>,
    actions: &mut Vec<EditorAction>,
) {
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
