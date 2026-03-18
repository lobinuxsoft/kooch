//! World panel — entity hierarchy list with context menu.

use std::any::TypeId;
use std::collections::HashSet;

use egui::NumExt as _;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};

/// Walks up from `entity` through the parent chain looking for `ancestor`.
/// Returns `true` if `entity` is a descendant of `ancestor` (cycle prevention).
fn is_descendant(entity: Entity, ancestor: Entity, entities: &[EntityDisplayInfo]) -> bool {
    let mut current = entity;
    loop {
        let parent = entities
            .iter()
            .find(|e| e.entity == current)
            .and_then(|e| e.parent);
        match parent {
            Some(p) => {
                if p == ancestor {
                    return true;
                }
                current = p;
            }
            None => return false,
        }
    }
}

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
            let indent_str = "  ".repeat(info.depth);
            let indented_label = format!("{indent_str}{label}");

            // Check if this entity is the one being dragged.
            let being_dragged = egui::DragAndDrop::payload::<Entity>(ui.ctx())
                .is_some_and(|p| *p == info.entity);

            // Custom selectable label with click + drag sensing (single widget
            // avoids the drag overlay stealing click events from selection).
            let button_padding = ui.spacing().button_padding;
            let total_extra = button_padding + button_padding;
            let wrap_width = ui.available_width() - total_extra.x;
            let text: egui::WidgetText = indented_label.as_str().into();
            let galley = text.into_galley(ui, None, wrap_width, egui::TextStyle::Button);
            let mut desired_size = total_extra + galley.size();
            desired_size.y = desired_size.y.at_least(ui.spacing().interact_size.y);
            let (rect, resp) =
                ui.allocate_at_least(desired_size, egui::Sense::click_and_drag());

            if ui.is_rect_visible(rect) {
                let text_pos = ui
                    .layout()
                    .align_size_within_rect(galley.size(), rect.shrink2(button_padding))
                    .min;
                let visuals = ui.style().interact_selectable(&resp, is_selected);
                if is_selected
                    || resp.hovered()
                    || resp.highlighted()
                    || resp.has_focus()
                {
                    let r = rect.expand(visuals.expansion);
                    ui.painter()
                        .rect(r, visuals.rounding, visuals.bg_fill, visuals.bg_stroke);
                }
                let mut text_color = visuals.text_color();
                if being_dragged {
                    text_color = egui::Color32::from_rgba_unmultiplied(
                        text_color.r(),
                        text_color.g(),
                        text_color.b(),
                        80,
                    );
                }
                ui.painter().galley(text_pos, galley, text_color);
            }

            // Single response handles both click and drag.
            resp.dnd_set_drag_payload(info.entity);

            // Drop target: highlight valid targets and push reparent action on release.
            if !being_dragged {
                if let Some(dragged) = resp.dnd_hover_payload::<Entity>() {
                    let d = *dragged;
                    if d != info.entity && !is_descendant(info.entity, d, entities) {
                        ui.painter().rect_filled(
                            resp.rect,
                            2.0,
                            egui::Color32::from_rgba_unmultiplied(60, 130, 230, 40),
                        );
                    }
                }
                if let Some(dragged) = resp.dnd_release_payload::<Entity>() {
                    let d = *dragged;
                    if d != info.entity && !is_descendant(info.entity, d, entities) {
                        actions.push(EditorAction::Reparent {
                            entity: d,
                            new_parent: Some(info.entity),
                        });
                    }
                }
            }

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

        // Empty space drop target — drop here to unparent an entity.
        let remaining = ui.available_rect_before_wrap();
        let empty_resp = ui.allocate_rect(remaining, egui::Sense::hover());
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
