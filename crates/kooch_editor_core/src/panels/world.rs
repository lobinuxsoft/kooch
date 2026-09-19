//! World panel — entity hierarchy list with context menu.

pub(crate) mod entity_row;
mod filter;
mod scene_bar;
mod spawn_menu;

use kooch_ecs::entity::Entity;

use crate::actions::EditorAction;
use crate::icons;
use crate::state::{EntityDisplayInfo, ReflectedTypeInfo, SceneDisplayInfo};

use self::entity_row::draw_entity_row;
use self::filter::{WorldFilter, draw_filter_bar};
use self::scene_bar::draw_scene_bar;
use self::spawn_menu::spawn_entries;

/// Content of the "World" tab — entity hierarchy list with context menu.
pub(crate) fn draw_world_content(
    ui: &mut egui::Ui,
    focused: bool,
    entities: &[EntityDisplayInfo],
    selected: &mut Vec<Entity>,
    pinned: &mut std::collections::HashSet<Entity>,
    reflected_types: &[ReflectedTypeInfo],
    actions: &mut Vec<EditorAction>,
    entity_count: usize,
    archetype_count: usize,
    active_archetype_count: usize,
    last_clicked_index: &mut Option<usize>,
    scenes: &[SceneDisplayInfo],
    clipboard_has_entities: bool,
) {
    draw_scene_bar(ui, scenes, actions);
    ui.label(format!(
        "{} entities, {} archetypes ({} active)",
        entity_count, archetype_count, active_archetype_count,
    ));
    ui.separator();

    ui.separator();

    // Read, drawn, written back. Held in egui's temp store rather than threaded through the
    // editor's state for the same reason the group flags are: it is view state of one panel and it
    // should not outlive the session. See `WorldFilter`.
    let filter_id = egui::Id::new("world_filter");
    let mut filter = ui.data_mut(|d| d.get_temp::<WorldFilter>(filter_id).unwrap_or_default());
    draw_filter_bar(ui, entities, &mut filter);
    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
    ui.separator();

    // Every line the panel will show, headers included, before any of them is drawn. Collapsed
    // scenes contribute their header and nothing else, so the list is exactly what is on screen and
    // its length is exactly what the scrollbar should describe.
    let focus = newly_focused(ui, selected);
    if let Some(focus) = focus {
        reveal_group_of(ui, entities, scenes, focus);
    }

    let rows = build_rows(ui, entities, scenes, &filter);
    // 🔴 What the panel is actually SHOWING, in the order it shows it — the filter applied,
    // collapsed subtrees left out.
    let listed: Vec<usize> = rows
        .iter()
        .filter_map(|row| match row {
            WorldRow::Entity(idx) => Some(*idx),
            _ => None,
        })
        .collect();
    // ⚠️ After the rows, so it can be told what is listed — which means a selection moved by the
    // keyboard is revealed and scrolled to on the NEXT frame rather than this one. One frame, and
    // only for the arrows: a click is handled inside the row, where the rows already exist.
    handle_keyboard(
        ui,
        focused,
        entities,
        &listed,
        selected,
        last_clicked_index,
        actions,
    );
    let row_h = entity_row::row_height(ui);
    let scroll_to = focus.and_then(|focus| scroll_offset_for(ui, &rows, entities, focus, row_h));

    // 🔴 Claimed BEFORE the list, over the whole panel, and that order is the fix rather than a
    // detail.
    let background_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        background_rect,
        ui.id().with("world_background"),
        egui::Sense::click(),
    );

    let mut area = egui::ScrollArea::vertical()
        .id_salt("world_tree")
        // Fills the panel instead of shrinking to whatever is drawn. A virtualized list only ever
        // draws the rows that fit, so letting it shrink leaves a gap under the last one and puts
        // the drop target for "unparent" somewhere other than the bottom of the panel.
        .auto_shrink([false; 2]);
    if let Some(offset) = scroll_to {
        // Applied on this frame only. Setting it every frame would pin
        // the list in place and there would be no scrolling by hand.
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, row_h, rows.len(), |ui, range| {
        // Recorded so the next selection can tell whether its row is already on screen. Written
        // here because this is the only place that knows it — and read by `scroll_offset_for`,
        // which is what stops a click on a visible row from yanking the list.
        ui.data_mut(|d| {
            d.insert_temp(visible_range_id(), (range.start, range.end));
        });

        // The range is the slice of rows that fits on screen — twenty of them, whether the scene
        // holds six hundred entities or sixty thousand.
        for index in range {
            match &rows[index] {
                WorldRow::Group(header) => {
                    draw_group_header(ui, header, row_h, clipboard_has_entities, selected, actions)
                }
                WorldRow::Note(text) => {
                    // Indented like the entities it stands in for, or the
                    // note explaining an empty scene sits further left
                    // than the rows it is about.
                    ui.weak(format!("    {text}"));
                }
                WorldRow::Entity(idx) => {
                    let info = &entities[*idx];
                    // A leaf gets `None` and no triangle. The default
                    // here has to match `push_members`' or the row would
                    // point one way while the list hid the other.
                    let subtree = (!info.children.is_empty())
                        .then(|| subtree_open(ui, info.entity, !info.is_prefab_instance));
                    draw_entity_row(
                        ui,
                        *idx,
                        info,
                        entities,
                        selected,
                        pinned,
                        reflected_types,
                        &listed,
                        clipboard_has_entities,
                        actions,
                        last_clicked_index,
                        subtree,
                    );
                }
            }
        }
    });

    if background.clicked() {
        selected.clear();
        *last_clicked_index = None;
    }

    // The same entries the toolbar's Spawn button offers, reached where people actually reach for
    // them: right-click in the empty part of the hierarchy. A row's own right-click menu handles
    // per-entity actions, including Add Component (#591).
    background.context_menu(|ui| {
        ui.set_min_width(240.0);
        ui.label("New scene");
        ui.separator();
        spawn_entries(ui, actions, crate::actions::SpawnTarget::NewScene);
        if ui
            .add_enabled(
                clipboard_has_entities,
                egui::Button::new(format!("{} Paste", icons::PACKAGE)),
            )
            .on_hover_text("Put what was copied into a scene of its own")
            .clicked()
        {
            actions.push(EditorAction::PasteEntities {
                into: crate::actions::SpawnTarget::NewScene,
            });
            ui.close();
        }
    });
    // A prefab dropped into the hierarchy spawns at the position it was authored at: a list of
    // names has no geometry to read a place out of, and defaulting to the origin would silently
    // move a prefab that was deliberately authored elsewhere.
    if background
        .dnd_hover_payload::<crate::drag_drop::DraggedAsset>()
        .is_some_and(|a| a.type_name == crate::drag_drop::PREFAB_TYPE_NAME)
    {
        ui.painter().rect_filled(
            background_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
        );
        if let Some(prefab) = background.dnd_release_payload::<crate::drag_drop::DraggedAsset>() {
            actions.push(EditorAction::InstantiatePrefab {
                prefab: prefab.guid,
                at: crate::viewport_pick::DropPoint::Authored,
            });
        }
    }
    if background.dnd_hover_payload::<Entity>().is_some() {
        ui.painter().rect_filled(
            background_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(100, 100, 100, 20),
        );
    }
    if let Some(dragged) = background.dnd_release_payload::<Entity>() {
        let d = *dragged;
        if entities.iter().any(|e| e.entity == d && e.parent.is_some()) {
            actions.push(EditorAction::Reparent {
                entity: d,
                new_parent: None,
            });
        }
    }
}

/// What the "Unsaved" group offers: somewhere to put something, and nothing about files.
fn unowned_entries(
    ui: &mut egui::Ui,
    clipboard_has_entities: bool,
    actions: &mut Vec<EditorAction>,
) {
    ui.menu_button("New", |ui| {
        spawn_entries(ui, actions, crate::actions::SpawnTarget::Active);
    });
    if ui
        .add_enabled(
            clipboard_has_entities,
            egui::Button::new(format!("{} Paste", icons::PACKAGE)),
        )
        .on_hover_text("Put what was copied into the active scene")
        .clicked()
    {
        actions.push(EditorAction::PasteEntities {
            into: crate::actions::SpawnTarget::Active,
        });
        ui.close();
    }
}

/// The colour of a scene that has edits not on disk.
const DIRTY_SCENE: egui::Color32 = egui::Color32::from_rgb(210, 150, 60);

/// The right-click menu on a scene's row.
fn scene_context_menu(
    resp: &egui::Response,
    header: &GroupHeader,
    clipboard_has_entities: bool,
    actions: &mut Vec<EditorAction>,
) {
    resp.context_menu(|ui| {
        ui.set_min_width(240.0);
        let Some(scene) = header.scene else {
            unowned_entries(ui, clipboard_has_entities, actions);
            return;
        };
        // 🔴 The only place the active scene can be chosen when a single scene is open:
        // `draw_scene_bar` hides itself under two scenes, so with one there was nothing on screen
        // naming it and nothing to click.
        match header.active {
            // Says so rather than offering nothing: a menu that is silent
            // about which scene is active leaves the question unanswered
            // in the one place it was asked.
            true => {
                ui.add_enabled(false, egui::Button::new("Active scene"));
            }
            false => {
                if ui
                    .button("Make Active")
                    .on_hover_text("New entities land in this scene")
                    .clicked()
                {
                    actions.push(EditorAction::SetActiveScene(scene));
                    ui.close();
                }
            }
        }
        ui.separator();
        // No icon on either.
        let save = ui.button("Save").on_hover_text(if header.dirty {
            "Write this scene back to its own file"
        } else {
            "This scene has no unsaved changes"
        });
        if save.clicked() {
            actions.push(EditorAction::SaveOpenScene(scene));
            ui.close();
        }
        if ui
            .button("Save As…")
            .on_hover_text("Write this scene to a new file and adopt it")
            .clicked()
        {
            actions.push(EditorAction::SaveOpenSceneAs(scene));
            ui.close();
        }
        // Only offered when there is something to discard, and only for a scene that has a file.
        // Without one there is nothing to revert *to*, and despawning its entities would delete
        // work rather than undo it — the one thing "discard" must never be mistaken for.
        if header.dirty && header.has_file {
            let discard = ui
                .button("Discard Changes")
                .on_hover_text("Throw away this scene's edits and read it back from its file");
            if discard.clicked() {
                actions.push(EditorAction::RevertOpenScene(scene));
                ui.close();
            }
        }
        ui.separator();
        // Into *this* scene. The toolbar's Spawn button authors into the
        // active one, which with several open is routinely not the scene
        // somebody just right-clicked.
        ui.menu_button("New", |ui| {
            spawn_entries(ui, actions, crate::actions::SpawnTarget::Scene(scene));
        });

        // Into *this* scene, for the same reason. Copying out of one
        // scene and pasting into another is the gesture that used to
        // leave the copies under "Unsaved" with nothing saying why.
        if ui
            .add_enabled(
                clipboard_has_entities,
                egui::Button::new(format!("{} Paste", icons::PACKAGE)),
            )
            .on_hover_text("Put what was copied into this scene")
            .clicked()
        {
            actions.push(EditorAction::PasteEntities {
                into: crate::actions::SpawnTarget::Scene(scene),
            });
            ui.close();
        }
        ui.separator();
        // 🔴 The only way to close ONE scene without the scene bar, which hides itself under two
        // scenes — so the moment additive opening gave you a second scene, closing either one had
        // no gesture but that bar.
        if ui
            .button("Close Scene")
            .on_hover_text(match header.dirty {
                true => "Close this scene — ITS UNSAVED EDITS ARE DISCARDED",
                false => "Close this scene, leaving the others open",
            })
            .clicked()
        {
            actions.push(EditorAction::CloseScene(scene));
            ui.close();
        }
    });
}

/// Keyboard shortcuts for the World panel: Delete, Ctrl+A, arrow up/down.
fn handle_keyboard(
    ui: &egui::Ui,
    focused: bool,
    entities: &[EntityDisplayInfo],
    // The display indices the panel is showing, in order.
    listed: &[usize],
    selected: &mut Vec<Entity>,
    last_clicked_index: &mut Option<usize>,
    actions: &mut Vec<EditorAction>,
) {
    // Navigation belongs to the panel with focus. Without this the arrows moved the hierarchy's
    // selection from inside the Console, and Ctrl+A fought the select-all of whatever text field
    // was being typed in (#661).
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
    if kb_select_all && !listed.is_empty() {
        // "All" means all of what is on screen. Under a filter it used to
        // mean all two thousand, which is the opposite of what filtering
        // was for.
        selected.clear();
        selected.extend(
            listed
                .iter()
                .filter_map(|&i| entities.get(i))
                .map(|e| e.entity),
        );
        *last_clicked_index = listed.last().copied();
    }

    // Keyboard navigation: Arrow Up/Down.
    let kb_up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
    let kb_down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
    let kb_shift = ui.input(|i| i.modifiers.shift);

    if (kb_up || kb_down) && !listed.is_empty() {
        // Stepping through the LISTED rows, not the display list, or an
        // arrow lands on an entity the filter removed and the panel shows
        // nothing moving.
        let here = last_clicked_index
            .and_then(|idx| listed.iter().position(|&i| i == idx))
            .unwrap_or(0);
        let next = match kb_up {
            true => here.saturating_sub(1),
            false => (here + 1).min(listed.len() - 1),
        };
        let new_idx = listed[next];

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

mod rows;

use rows::*;

#[cfg(test)]
mod tests;
