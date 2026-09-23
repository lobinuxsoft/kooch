//! Entity row rendering for the World panel: indented label, drag/drop source + target, click
//! selection (with Shift / Ctrl modifiers), and the right-click context menu for despawn /
//! add-component / remove- component.

use std::collections::HashSet;

use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;
use crate::drag_drop::DraggedComponent;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo};
use crate::widgets::SelectableRow;

/// Height of one row in the hierarchy, in points.
pub(super) use crate::widgets::row_height;

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

/// Renders a single entity row. Mutates `selected` and `last_clicked_index` on click; pushes
/// [`EditorAction`]s for context-menu operations and drag/drop. How many levels of indent a row at
/// `depth` sits at.
pub(super) fn indent_levels(depth: usize) -> usize {
    depth + 1
}

/// Where this row's disclosure triangle goes, or `None` when the row is too narrow to hold one.
fn twisty_rect(ui: &egui::Ui, resp: &egui::Response, depth: usize) -> Option<egui::Rect> {
    let icon_width = ui.spacing().icon_width;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let space = ui.fonts_mut(|fonts| fonts.glyph_width(&font, ' '));
    let left = resp.rect.left() + space * (indent_levels(depth) * 2) as f32;
    let rect = egui::Rect::from_center_size(
        egui::pos2(left + icon_width * 0.5, resp.rect.center().y),
        egui::vec2(icon_width, icon_width),
    );
    resp.rect.contains_rect(rect).then_some(rect)
}

pub(super) fn draw_entity_row(
    ui: &mut egui::Ui,
    idx: usize,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    pinned: &mut HashSet<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    // The display indices the panel is showing, in order — see
    // `listed_range`.
    listed: &[usize],
    clipboard_has_entities: bool,
    actions: &mut Vec<EditorAction>,
    last_clicked_index: &mut Option<usize>,
    // `Some(open)` when this entity has children, `None` when it is a
    // leaf. Decided by the caller, which is also what builds the row
    // list — the two have to agree about what is hidden.
    subtree: Option<bool>,
) {
    let display_name = display_name_for(info);
    let mut label = build_label(info, display_name.as_deref());
    // A pin has to be visible from the row, or it is a state you can
    // enter and then forget you are in.
    if pinned.contains(&info.entity) {
        label = format!("{label} {}", icons::EYE);
    }
    let is_selected = selected.contains(&info.entity);

    let indent_str = "  ".repeat(indent_levels(info.depth));
    // Two more spaces for the disclosure triangle, on every row and not only the ones that have
    // one: without them a leaf's text sits two characters left of its siblings' and the column
    // reads as ragged depth that is not there.
    let indented_label = format!("{indent_str}  {label}");

    // Check if this entity is the one being dragged.
    let being_dragged =
        egui::DragAndDrop::payload::<Entity>(ui.ctx()).is_some_and(|p| *p == info.entity);

    // Click and drag on one response: two widgets would let the drag
    // overlay steal the click that selects the row.
    // A prefab instance reads as one at a glance, as it does in Unity and Godot: the blue says the
    // entity follows a file, and that editing it here is an override.
    let text = match info.is_prefab_instance {
        true => egui::RichText::new(indented_label.as_str()).color(crate::palette::family::PREFAB),
        false => egui::RichText::new(indented_label.as_str()),
    };
    let resp = SelectableRow::new(text)
        .selected(is_selected)
        .sense(egui::Sense::click_and_drag())
        .dimmed(being_dragged)
        .show(ui);

    // 🔴 The triangle is painted onto the row and hit-tested out of the row's own response, rather
    // than being a widget of its own.
    if let Some(open) = subtree
        && let Some(twisty) = twisty_rect(ui, &resp, info.depth)
    {
        let mut icon_resp = resp.clone();
        icon_resp.rect = twisty;
        egui::collapsing_header::paint_default_icon(ui, if open { 1.0 } else { 0.0 }, &icon_resp);
        // A click that landed on the triangle toggles and selects
        // nothing. Returning early is what keeps it from doing both.
        if resp.clicked()
            && resp
                .interact_pointer_pos()
                .is_some_and(|at| twisty.x_range().contains(at.x))
        {
            ui.data_mut(|data| data.insert_persisted(super::subtree_id(info.entity), !open));
            return;
        }
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
        listed,
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
        clipboard_has_entities,
        actions,
    );
}

pub(crate) fn display_name_for(info: &EntityDisplayInfo) -> Option<String> {
    info.components
        .iter()
        .find(|c| c.short_name == "Name")
        .and_then(|c| c.fields.values())
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
    // Drop target: guard each `release_payload` call by a prior `hover_payload::<T>` check of the
    // same type.
    if !being_dragged && let Some(dragged) = resp.dnd_hover_payload::<Entity>() {
        let d = *dragged;
        if d != info.entity && !is_descendant(info.entity, d, entities) {
            let intent = drop_intent(ui, resp);
            match intent {
                // Onto the row: become its child.
                DropIntent::Into => {
                    ui.painter().rect_filled(
                        resp.rect,
                        2.0,
                        egui::Color32::from_rgba_unmultiplied(60, 130, 230, 40),
                    );
                }
                // Between two rows: become a sibling, at that spot.
                DropIntent::Before | DropIntent::After => {
                    let y = match intent {
                        DropIntent::Before => resp.rect.top(),
                        _ => resp.rect.bottom(),
                    };
                    let left = resp.rect.left() + sibling_indent(ui, info.depth);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(left, y - 1.5),
                            egui::pos2(resp.rect.right(), y + 1.5),
                        ),
                        1.0,
                        egui::Color32::from_rgb(90, 160, 245),
                    );
                }
            }
            if let Some(released) = resp.dnd_release_payload::<Entity>() {
                let r = *released;
                if r != info.entity && !is_descendant(info.entity, r, entities) {
                    if intent != DropIntent::Into {
                        // 🔴 Its siblings, not its children. Dropping in the gap between two rows
                        // means "beside them" — which is also the only gesture that can take an
                        // entity *out* of a parent, since every row's middle already means "into".
                        actions.push(EditorAction::MoveEntity {
                            entity: r,
                            new_parent: info.parent,
                            before: match intent {
                                DropIntent::Before => Some(info.entity),
                                _ => next_sibling(info, entities),
                            },
                        });
                        if let Some(parent) = info.parent {
                            reveal_chain(ui, parent, entities);
                        }
                        return;
                    }
                    actions.push(EditorAction::Reparent {
                        entity: r,
                        new_parent: Some(info.entity),
                    });
                    // 🔴 Open the new parent and everything above it, right up to the root. Dropping
                    // onto a collapsed entity otherwise makes the dragged one *vanish*: the
                    // reparent works, and its row is inside a subtree that is not listed.
                    reveal_chain(ui, info.entity, entities);
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

/// The rows a Shift+Click spans, as display indices.
pub(super) fn listed_range(listed: &[usize], anchor: usize, idx: usize) -> Option<&[usize]> {
    let from = listed.iter().position(|&i| i == anchor)?;
    let here = listed.iter().position(|&i| i == idx)?;
    Some(&listed[from.min(here)..=from.max(here)])
}

fn handle_click(
    resp: &egui::Response,
    ui: &egui::Ui,
    idx: usize,
    info: &EntityDisplayInfo,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    listed: &[usize],
    last_clicked_index: &mut Option<usize>,
    is_selected: bool,
) {
    if !resp.clicked() {
        return;
    }
    let modifiers = ui.input(|i| i.modifiers);
    if modifiers.shift {
        // Shift+Click: range selection from anchor to current, over the
        // rows on screen. See `listed_range`.
        let anchor = last_clicked_index.unwrap_or(idx);
        let span = listed_range(listed, anchor, idx);
        if !modifiers.ctrl && !modifiers.command {
            selected.clear();
        }
        // No span means the anchor is not listed any more, so there is
        // nothing to reach across: this is a click on one row.
        for &i in span.unwrap_or(std::slice::from_ref(&idx)) {
            let Some(entity) = entities.get(i).map(|e| e.entity) else {
                continue;
            };
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

/// Opens `entity` and every ancestor of it, so a child dropped onto it has a row.
pub(super) fn reveal_chain(ui: &egui::Ui, entity: Entity, entities: &[EntityDisplayInfo]) {
    let mut at = Some(entity);
    while let Some(current) = at {
        ui.data_mut(|data| data.insert_persisted(super::subtree_id(current), true));
        at = entities
            .iter()
            .find(|e| e.entity == current)
            .and_then(|e| e.parent);
    }
}

/// What a drop on this row means, from where the pointer is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropIntent {
    /// In the gap above: a sibling, in front of this row.
    Before,
    /// On the row itself: a child of it.
    Into,
    /// In the gap below: a sibling, behind this row.
    After,
}

/// Splits a row into an insert band, a parent band, and an insert band.
fn drop_intent(ui: &egui::Ui, resp: &egui::Response) -> DropIntent {
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return DropIntent::Into;
    };
    let band = resp.rect.height() * 0.25;
    if pointer.y < resp.rect.top() + band {
        DropIntent::Before
    } else if pointer.y > resp.rect.bottom() - band {
        DropIntent::After
    } else {
        DropIntent::Into
    }
}

/// Where the insertion bar starts: level with the row's own label, so it
/// shows which *depth* the entity would land at, not only which gap.
fn sibling_indent(ui: &egui::Ui, depth: usize) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let space = ui.fonts_mut(|fonts| fonts.glyph_width(&font, ' '));
    space * (indent_levels(depth) * 2) as f32
}

/// The row after `info` among its own siblings, or `None` if it is last.
fn next_sibling(info: &EntityDisplayInfo, entities: &[EntityDisplayInfo]) -> Option<Entity> {
    let mut siblings = entities.iter().filter(|e| e.parent == info.parent);
    siblings.find(|e| e.entity == info.entity)?;
    siblings.next().map(|e| e.entity)
}

/// Where a row's root-level spawn goes, and what to call it.
pub(super) fn root_target(
    scene: Option<kooch_core::Guid>,
) -> (&'static str, crate::actions::SpawnTarget) {
    match scene {
        Some(scene) => (
            "New in This Scene",
            crate::actions::SpawnTarget::Scene(scene),
        ),
        None => ("New Beside This", crate::actions::SpawnTarget::Active),
    }
}

mod context_menu;

use super::spawn_entries;
use context_menu::*;
