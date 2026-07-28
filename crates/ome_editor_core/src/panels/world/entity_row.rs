//! Entity row rendering for the World panel: indented label, drag/drop
//! source + target, click selection (with Shift / Ctrl modifiers), and
//! the right-click context menu for despawn / add-component / remove-
//! component.

use std::collections::HashSet;

use egui::NumExt as _;
use ome_ecs::component::ComponentId;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::drag_drop::DraggedComponent;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};

/// Walks up from `entity` through the parent chain looking for `ancestor`.
/// Returns `true` if `entity` is a descendant of `ancestor` (cycle prevention).
pub(super) fn is_descendant(
    entity: Entity,
    ancestor: Entity,
    entities: &[EntityDisplayInfo],
) -> bool {
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

/// Renders a single entity row. Mutates `selected` and `last_clicked_index`
/// on click; pushes [`EditorAction`]s for context-menu operations and drag/drop.
pub(super) fn draw_entity_row(
    ui: &mut egui::Ui,
    idx: usize,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    last_clicked_index: &mut Option<usize>,
) {
    let display_name = display_name_for(info);
    let label = build_label(info, display_name.as_deref());
    let is_selected = selected.contains(&info.entity);

    let indent_str = "  ".repeat(info.depth);
    let indented_label = format!("{indent_str}{label}");

    // Check if this entity is the one being dragged.
    let being_dragged =
        egui::DragAndDrop::payload::<Entity>(ui.ctx()).is_some_and(|p| *p == info.entity);

    // Custom selectable label with click + drag sensing (single widget
    // avoids the drag overlay stealing click events from selection).
    let button_padding = ui.spacing().button_padding;
    let total_extra = button_padding + button_padding;
    let wrap_width = ui.available_width() - total_extra.x;
    let text: egui::WidgetText = indented_label.as_str().into();
    let galley = text.into_galley(ui, None, wrap_width, egui::TextStyle::Button);
    let mut desired_size = total_extra + galley.size();
    desired_size.y = desired_size.y.at_least(ui.spacing().interact_size.y);
    let (rect, resp) = ui.allocate_at_least(desired_size, egui::Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        let text_pos = ui
            .layout()
            .align_size_within_rect(galley.size(), rect.shrink2(button_padding))
            .min;
        let visuals = ui.style().interact_selectable(&resp, is_selected);
        if is_selected || resp.hovered() || resp.highlighted() || resp.has_focus() {
            let r = rect.expand(visuals.expansion);
            ui.painter().rect(
                r,
                visuals.corner_radius,
                visuals.bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
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

    handle_drop_targets(ui, &resp, info, entities, actions, being_dragged);
    handle_click(
        &resp,
        ui,
        idx,
        info,
        entities,
        selected,
        last_clicked_index,
        is_selected,
    );
    handle_context_menu(&resp, info, entities, selected, reflected_types, actions);
}

pub(crate) fn display_name_for(info: &EntityDisplayInfo) -> Option<String> {
    info.components
        .iter()
        .find(|c| c.short_name == "Name")
        .and_then(|c| c.fields.as_ref())
        .and_then(|fields| {
            fields.iter().find_map(|(name, val)| {
                if name == "value"
                    && let ReflectValue::String(s) = val
                    && !s.is_empty()
                {
                    return Some(s.clone());
                }
                None
            })
        })
}

fn build_label(info: &EntityDisplayInfo, display_name: Option<&str>) -> String {
    let has_children = !info.children.is_empty();
    let icon = if has_children {
        icons::TREE_STRUCTURE
    } else {
        icons::CUBE
    };

    if let Some(name) = display_name {
        format!("{} {}  [{}]", icon, name, info.components.len())
    } else {
        format!(
            "{} Entity {}:{}  [{}]",
            icon,
            info.entity.index(),
            info.entity.generation(),
            info.components.len(),
        )
    }
}

fn handle_drop_targets(
    ui: &mut egui::Ui,
    resp: &egui::Response,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    actions: &mut Vec<EditorAction>,
    being_dragged: bool,
) {
    // Drop target: guard each `release_payload` call by a prior
    // `hover_payload::<T>` check of the same type. `release_payload`
    // internally calls `take` *before* checking the type, so a
    // mismatched-type release silently drops the payload for any
    // subsequent check — meaning unguarded order-dependent checks
    // for multiple payload types on the same response are broken.
    if !being_dragged && let Some(dragged) = resp.dnd_hover_payload::<Entity>() {
        let d = *dragged;
        if d != info.entity && !is_descendant(info.entity, d, entities) {
            ui.painter().rect_filled(
                resp.rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(60, 130, 230, 40),
            );
            if let Some(released) = resp.dnd_release_payload::<Entity>() {
                let r = *released;
                if r != info.entity && !is_descendant(info.entity, r, entities) {
                    actions.push(EditorAction::Reparent {
                        entity: r,
                        new_parent: Some(info.entity),
                    });
                }
            }
        }
    }

    // Drop target: Components-panel drag. Accept on any entity row.
    if resp.dnd_hover_payload::<DraggedComponent>().is_some() {
        ui.painter().rect_filled(
            resp.rect,
            2.0,
            egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
        );
        if let Some(dragged) = resp.dnd_release_payload::<DraggedComponent>() {
            actions.push(EditorAction::AddComponent {
                entity: info.entity,
                component: dragged.0,
            });
        }
    }
}

fn handle_click(
    resp: &egui::Response,
    ui: &egui::Ui,
    idx: usize,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    last_clicked_index: &mut Option<usize>,
    is_selected: bool,
) {
    if !resp.clicked() {
        return;
    }
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

fn handle_context_menu(
    resp: &egui::Response,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
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
            ui.close();
        }

        // Add Component submenu (only for single entity).
        if selected.len() == 1 {
            let entity = selected[0];
            let existing: HashSet<ComponentId> = entities
                .iter()
                .find(|e| e.entity == entity)
                .map(|e| e.components.iter().map(|c| c.component).collect())
                .unwrap_or_default();

            let available: Vec<&ReflectedTypeInfo> = reflected_types
                .iter()
                .filter(|t| !existing.contains(&t.component))
                .collect();

            if !available.is_empty() {
                ui.menu_button(format!("{} Add Component", icons::PLUS), |ui| {
                    crate::panels::add_component_menu::draw_categorized(
                        ui,
                        &available,
                        |component| {
                            actions.push(EditorAction::AddComponent { entity, component });
                        },
                    );
                });
            }
        } else if selected.len() > 1 {
            // Multi-select: add component to all selected.
            let all: Vec<&ReflectedTypeInfo> = reflected_types.iter().collect();
            ui.menu_button(format!("{} Add Component to all", icons::PLUS), |ui| {
                crate::panels::add_component_menu::draw_categorized(ui, &all, |component| {
                    for &entity in selected.iter() {
                        actions.push(EditorAction::AddComponent { entity, component });
                    }
                });
            });

            // Multi-select: remove shared component from all selected.
            // Collect components present in ALL selected entities.
            let selected_infos: Vec<&EntityDisplayInfo> = entities
                .iter()
                .filter(|e| selected.contains(&e.entity))
                .collect();

            if !selected_infos.is_empty() {
                let mut shared: Vec<(ComponentId, String)> = selected_infos[0]
                    .components
                    .iter()
                    .filter(|c| {
                        selected_infos[1..].iter().all(|info| {
                            info.components.iter().any(|ic| ic.component == c.component)
                        })
                    })
                    .map(|c| (c.component, c.short_name.clone()))
                    .collect();
                shared.sort_by(|a, b| a.1.cmp(&b.1));

                if !shared.is_empty() {
                    ui.menu_button(
                        format!("{} Remove Component from all", icons::MINUS),
                        |ui| {
                            for (component, name) in &shared {
                                if ui.selectable_label(false, name).clicked() {
                                    for &entity in selected.iter() {
                                        actions.push(EditorAction::RemoveComponent {
                                            entity,
                                            component: *component,
                                        });
                                    }
                                    ui.close();
                                }
                            }
                        },
                    );
                }
            }
        }
    });
}
