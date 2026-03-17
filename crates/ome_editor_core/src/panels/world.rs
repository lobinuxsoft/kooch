//! World panel — entity hierarchy list with context menu.

use std::any::TypeId;
use std::collections::HashSet;

use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};

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
) {
    ui.label(format!(
        "{} entities, {} archetypes ({} active)",
        entity_count, archetype_count, active_archetype_count,
    ));
    ui.separator();

    ui.horizontal(|ui| {
        if ui.button(format!("{} Spawn", icons::PLUS)).clicked() {
            actions.push(EditorAction::Spawn);
        }
        let can_despawn = !selected.is_empty();
        if ui
            .add_enabled(
                can_despawn,
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

    // Delete/Suprimir: despawn selected entities.
    let kb_delete = ui.input(|i| i.key_pressed(egui::Key::Delete));
    if kb_delete && !selected.is_empty() {
        for entity in selected.drain(..) {
            actions.push(EditorAction::Despawn(entity));
        }
        *last_clicked_index = None;
    }

    // Keyboard navigation: Ctrl+A to select all.
    let kb_select_all = ui.input(|i| {
        i.modifiers.command && i.key_pressed(egui::Key::A)
    });
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

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (idx, info) in entities.iter().enumerate() {
            // If the entity has a Name component with a non-empty value,
            // display that instead of the raw index:generation.
            let display_name = info
                .components
                .iter()
                .find(|c| c.short_name == "Name")
                .and_then(|c| c.fields.as_ref())
                .and_then(|fields| {
                    fields.iter().find_map(|(name, val)| {
                        if name == "value" {
                            if let ReflectValue::String(s) = val {
                                if !s.is_empty() {
                                    return Some(s.clone());
                                }
                            }
                        }
                        None
                    })
                });

            let has_children = !info.children.is_empty();
            let icon = if has_children {
                icons::TREE_STRUCTURE
            } else {
                icons::CUBE
            };

            let label = if let Some(name) = &display_name {
                format!(
                    "{} {}  [{}]",
                    icon,
                    name,
                    info.components.len()
                )
            } else {
                format!(
                    "{} Entity {}:{}  [{}]",
                    icon,
                    info.entity.index(),
                    info.entity.generation(),
                    info.components.len()
                )
            };
            let is_selected = selected.contains(&info.entity);

            // Indent based on hierarchy depth.
            let indent = info.depth as f32 * 16.0;
            let resp = ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.selectable_label(is_selected, &label)
            }).inner;

            if resp.clicked() {
                let modifiers = ui.input(|i| i.modifiers);
                if modifiers.shift {
                    // Shift+Click: range selection from anchor to current.
                    let anchor = last_clicked_index.unwrap_or(0);
                    let range_start = anchor.min(idx);
                    let range_end = anchor.max(idx);
                    if !modifiers.ctrl && !modifiers.command {
                        selected.clear();
                    }
                    for i in range_start..=range_end {
                        let entity = entities[i].entity;
                        if !selected.contains(&entity) {
                            selected.push(entity);
                        }
                    }
                    // Don't update anchor on Shift+Click — keep the original.
                } else if modifiers.ctrl || modifiers.command {
                    // Ctrl+Click: toggle individual item.
                    if is_selected {
                        selected.retain(|e| *e != info.entity);
                    } else {
                        selected.push(info.entity);
                    }
                    *last_clicked_index = Some(idx);
                } else {
                    // Plain click: replace selection.
                    selected.clear();
                    selected.push(info.entity);
                    *last_clicked_index = Some(idx);
                }
            }

            // Right click: context menu.
            resp.context_menu(|ui| {
                // Ensure the right-clicked entity is selected.
                if !selected.contains(&info.entity) {
                    selected.clear();
                    selected.push(info.entity);
                }

                let count = selected.len();
                let label = if count == 1 {
                    format!("{} Despawn", icons::TRASH)
                } else {
                    format!("{} Despawn {} entities", icons::TRASH, count)
                };

                if ui.button(label).clicked() {
                    for entity in selected.drain(..) {
                        actions.push(EditorAction::Despawn(entity));
                    }
                    ui.close_menu();
                }

                // Add Component submenu (only for single entity).
                if selected.len() == 1 {
                    let entity = selected[0];
                    let existing: HashSet<TypeId> = entities
                        .iter()
                        .find(|e| e.entity == entity)
                        .map(|e| e.components.iter().map(|c| c.type_id).collect())
                        .unwrap_or_default();

                    let available: Vec<&ReflectedTypeInfo> = reflected_types
                        .iter()
                        .filter(|t| !existing.contains(&t.type_id))
                        .collect();

                    if !available.is_empty() {
                        ui.menu_button(
                            format!("{} Add Component", icons::PLUS),
                            |ui| {
                                for type_info in &available {
                                    if ui
                                        .selectable_label(false, &type_info.short_name)
                                        .clicked()
                                    {
                                        actions.push(EditorAction::AddComponent {
                                            entity,
                                            type_id: type_info.type_id,
                                        });
                                        ui.close_menu();
                                    }
                                }
                            },
                        );
                    }
                } else if selected.len() > 1 {
                    // Multi-select: add component to all selected.
                    ui.menu_button(
                        format!("{} Add Component to all", icons::PLUS),
                        |ui| {
                            for type_info in reflected_types {
                                if ui
                                    .selectable_label(false, &type_info.short_name)
                                    .clicked()
                                {
                                    for &entity in selected.iter() {
                                        actions.push(EditorAction::AddComponent {
                                            entity,
                                            type_id: type_info.type_id,
                                        });
                                    }
                                    ui.close_menu();
                                }
                            }
                        },
                    );

                    // Multi-select: remove shared component from all selected.
                    // Collect components present in ALL selected entities.
                    let selected_infos: Vec<&EntityDisplayInfo> = entities
                        .iter()
                        .filter(|e| selected.contains(&e.entity))
                        .collect();

                    if !selected_infos.is_empty() {
                        let mut shared: Vec<(TypeId, String)> = selected_infos[0]
                            .components
                            .iter()
                            .filter(|c| {
                                selected_infos[1..].iter().all(|info| {
                                    info.components.iter().any(|ic| ic.type_id == c.type_id)
                                })
                            })
                            .map(|c| (c.type_id, c.short_name.clone()))
                            .collect();
                        shared.sort_by(|a, b| a.1.cmp(&b.1));

                        if !shared.is_empty() {
                            ui.menu_button(
                                format!("{} Remove Component from all", icons::MINUS),
                                |ui| {
                                    for (type_id, name) in &shared {
                                        if ui.selectable_label(false, name).clicked() {
                                            for &entity in selected.iter() {
                                                actions.push(EditorAction::RemoveComponent {
                                                    entity,
                                                    type_id: *type_id,
                                                });
                                            }
                                            ui.close_menu();
                                        }
                                    }
                                },
                            );
                        }
                    }
                }
            });
        }
    });
}
