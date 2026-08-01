//! Entity row rendering for the World panel: indented label, drag/drop
//! source + target, click selection (with Shift / Ctrl modifiers), and
//! the right-click context menu for despawn / add-component / remove-
//! component.

use std::collections::HashSet;

use egui::NumExt as _;
use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::drag_drop::DraggedComponent;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};

/// Height of one row in the hierarchy, in points.
///
/// Virtualizing the list means egui is told how tall the rows are before
/// any of them is drawn — it has to place a scrollbar for six hundred
/// entities while laying out the twenty that fit on screen. So this is
/// the **single** definition: the list asks it how much space to reserve
/// and the row asks it how much to occupy. Two copies of this formula
/// would drift by a pixel per row and the scrollbar would lie about
/// where it is by the bottom of a long scene.
///
/// Rows are exactly one line because [`draw_entity_row`] truncates
/// rather than wraps. A wrapped name would make its own row taller than
/// this and desynchronize the whole list below it.
pub(super) fn row_height(ui: &egui::Ui) -> f32 {
    let line = ui.text_style_height(&egui::TextStyle::Button);
    (line + 2.0 * ui.spacing().button_padding.y).at_least(ui.spacing().interact_size.y)
        + ui.spacing().item_spacing.y
}

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
    pinned: &mut HashSet<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    last_clicked_index: &mut Option<usize>,
) {
    let display_name = display_name_for(info);
    let mut label = build_label(info, display_name.as_deref());
    // A pin has to be visible from the row, or it is a state you can
    // enter and then forget you are in.
    if pinned.contains(&info.entity) {
        label = format!("{label} {}", icons::EYE);
    }
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
    // Truncate, never wrap. A wrapped name would be taller than
    // [`row_height`] promised, and every row below it in a virtualized
    // list would be drawn where the scrollbar says the previous one
    // ended rather than where it actually did. Truncating is also what
    // a hierarchy panel should do with a long name: the tree structure
    // is what is being read, not the full string.
    let galley = text.into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        wrap_width,
        egui::TextStyle::Button,
    );
    // Full width, not the width of the name. A row is a target — for a
    // click, for a drop, for the highlight that says which entity is
    // selected — and a target that stops where the text stops leaves
    // most of the panel dead to the pointer.
    let desired_size = egui::vec2(
        ui.available_width(),
        row_height(ui) - ui.spacing().item_spacing.y,
    );
    let (rect, resp) = ui.allocate_at_least(desired_size, egui::Sense::click_and_drag());

    if ui.is_rect_visible(rect) {
        // Left-aligned and vertically centred, stated rather than
        // inherited from the layout: now that the row is as wide as the
        // panel, asking the layout where to put the text would centre a
        // short name in the middle of a wide row and lose the
        // indentation that shows the hierarchy.
        let inner = rect.shrink2(button_padding);
        let text_pos = egui::pos2(inner.left(), inner.center().y - galley.size().y * 0.5);
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
    handle_context_menu(
        &resp,
        info,
        entities,
        selected,
        pinned,
        reflected_types,
        actions,
    );
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
    pinned: &mut HashSet<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
) {
    resp.context_menu(|ui| {
        // Ensure the right-clicked entity is selected.
        if !selected.contains(&info.entity) {
            selected.clear();
            selected.push(info.entity);
        }

        // Pinning is what makes a gizmo answerable without keeping the
        // entity selected — you pin the camera you are aiming, then go
        // move the thing it follows.
        //
        // Applied to the whole selection, because pinning six colliders
        // one at a time to compare them is the case you would want it
        // for.
        let all_pinned = selected.iter().all(|e| pinned.contains(e));
        let pin_label = match (all_pinned, selected.len()) {
            (true, 1) => format!("{} Unpin gizmos", icons::EYE),
            (true, n) => format!("{} Unpin gizmos ({n})", icons::EYE),
            (false, 1) => format!("{} Pin gizmos", icons::EYE),
            (false, n) => format!("{} Pin gizmos ({n})", icons::EYE),
        };
        if ui
            .button(pin_label)
            .on_hover_text("Keep this entity's gizmos drawn while something else is selected")
            .clicked()
        {
            for entity in selected.iter() {
                if all_pinned {
                    pinned.remove(entity);
                } else {
                    pinned.insert(*entity);
                }
            }
            ui.close();
        }
        ui.separator();

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

        // Only on an instance. Reverting is the operation that makes an
        // override safe to have: without it an accidental gizmo drag
        // detaches that transform from the prefab forever, and the only
        // way back is deleting the instance and placing a new one.
        if selected.len() == 1 && info.is_prefab_instance {
            let entity = selected[0];
            if ui
                .button(format!("{} Revert to Prefab", icons::ARROWS_CLOCKWISE))
                .on_hover_text("Drop this instance's changes and follow the prefab again")
                .clicked()
            {
                actions.push(EditorAction::RevertToPrefab {
                    entity,
                    component: None,
                });
                ui.close();
            }
        }

        // One entity only. A prefab is one tree with one root — see
        // `SceneDocument::root_index` — so N selected entities are either N
        // prefabs or one thing that is not a tree, and neither is what this
        // menu item means.
        if selected.len() == 1 {
            let entity = selected[0];
            // The same glyph the asset tree shows for a prefab file, since
            // that is what this produces.
            if ui
                .button(format!("{} Save as Prefab", icons::PACKAGE))
                .on_hover_text("Write this entity and its children to a scene file in assets/")
                .clicked()
            {
                actions.push(EditorAction::SavePrefab {
                    entity,
                    dest: None,
                    overwrite: false,
                });
                ui.close();
            }
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
