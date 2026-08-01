//! World panel — entity hierarchy list with context menu.

pub(crate) mod entity_row;
mod scene_bar;
mod spawn_menu;

use kooch_ecs::entity::Entity;

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
    pinned: &mut std::collections::HashSet<Entity>,
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

    // Every line the panel will show, headers included, before any of
    // them is drawn. Collapsed scenes contribute their header and
    // nothing else, so the list is exactly what is on screen and its
    // length is exactly what the scrollbar should describe.
    let rows = build_rows(ui, entities, scenes);
    let row_h = entity_row::row_height(ui);

    egui::ScrollArea::vertical()
        .id_salt("world_tree")
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            // The range is the slice of rows that fits on screen — twenty
            // of them, whether the scene holds six hundred entities or
            // sixty thousand. Everything a row costs (laying out its
            // text, sensing clicks, registering a drop target, walking
            // the parent chain to reject a cyclic reparent) is paid for
            // the rows a person can see rather than for the ones the
            // scroll position happens to be nowhere near.
            let at_end = range.end >= rows.len();
            for index in range {
                match &rows[index] {
                    WorldRow::Group(header) => draw_group_header(ui, header, row_h),
                    WorldRow::Note(text) => {
                        ui.weak(text);
                    }
                    WorldRow::Entity(idx) => {
                        draw_entity_row(
                            ui,
                            *idx,
                            &entities[*idx],
                            entities,
                            selected,
                            pinned,
                            reflected_types,
                            actions,
                            last_clicked_index,
                        );
                    }
                }
            }

            // Empty space: click to deselect, right-click to create, drop
            // target to unparent an entity.
            //
            // Only once the last row has been drawn, and only across what
            // is genuinely left over. `available_rect_before_wrap` is the
            // wrong question inside a virtualized list: the rows that are
            // scrolled past still hold their space, so "what is left"
            // includes theirs, and claiming it drew this target beside
            // the list rather than under it.
            if !at_end {
                return;
            }
            let remaining = egui::Rect::from_min_max(
                egui::pos2(ui.max_rect().left(), ui.cursor().top()),
                ui.max_rect().max,
            );
            if remaining.height() < 1.0 || remaining.width() < 1.0 {
                return;
            }
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
            // Filtered by type the way an Inspector asset slot is: a mesh
            // dragged over the hierarchy is not something to instance.
            if empty_resp
                .dnd_hover_payload::<crate::drag_drop::DraggedAsset>()
                .is_some_and(|a| a.type_name == crate::drag_drop::PREFAB_TYPE_NAME)
            {
                ui.painter().rect_filled(
                    remaining,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
                );
                if let Some(prefab) =
                    empty_resp.dnd_release_payload::<crate::drag_drop::DraggedAsset>()
                {
                    actions.push(EditorAction::InstantiatePrefab {
                        prefab: prefab.guid,
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

/// One line of the hierarchy, as the virtualized list addresses it.
///
/// The list is built every frame rather than cached: it is derived from
/// the scene set and the open/closed flags, and a cache of it would be a
/// second copy of state that can disagree with the first. Building it is
/// a walk over the entities with no per-row layout, text shaping or
/// interaction — the part that was expensive is drawing them, and that
/// is what the range restricts.
enum WorldRow {
    /// A group's header. Clicking it opens or closes the group.
    Group(GroupHeader),
    /// An entity, by index into the flat display list. The index is the
    /// one keyboard navigation and shift-range selection already use, so
    /// virtualizing changed nothing about what a row *is*.
    Entity(usize),
    /// Explanatory line under a header.
    Note(String),
}

/// A collapsible group's header line: the scenes, and the pseudo-group
/// holding entities that belong to none of them.
struct GroupHeader {
    /// What identifies the group across frames. Scenes use their id; the
    /// unsaved group has no scene to name it, hence the string.
    id: egui::Id,
    label: String,
    /// Whether it starts open the first time it is ever seen.
    default_open: bool,
}

impl GroupHeader {
    fn scene(scene: &SceneDisplayInfo, count: usize) -> Self {
        let dirty = if scene.dirty { " *" } else { "" };
        Self {
            id: egui::Id::new(("world_group_open", scene.id)),
            label: format!("{} ({count} entities){dirty}", scene.name),
            // The active scene starts expanded: it is the one being
            // worked in.
            default_open: scene.active,
        }
    }

    fn unsaved(count: usize) -> Self {
        Self {
            id: egui::Id::new("world_group_open_unsaved"),
            label: format!("Unsaved ({count} entities)"),
            default_open: true,
        }
    }

    /// Whether the group is open, remembered across frames.
    ///
    /// Kept in egui's own persisted store rather than in the editor's
    /// state: it is view state of one panel, it should survive a frame
    /// and not a project, and threading it through would put a field
    /// about a scrollbar into the editor's model of the world.
    fn is_open(&self, ui: &egui::Ui) -> bool {
        ui.data_mut(|data| *data.get_persisted_mut_or_insert_with(self.id, || self.default_open))
    }
}

/// Flattens the hierarchy into the lines the panel will show.
fn build_rows(
    ui: &egui::Ui,
    entities: &[EntityDisplayInfo],
    scenes: &[SceneDisplayInfo],
) -> Vec<WorldRow> {
    // One scene: no headers, since every row would sit under the same
    // one and the grouping would only cost a level of indentation.
    if scenes.len() < 2 {
        return (0..entities.len()).map(WorldRow::Entity).collect();
    }

    let mut rows = Vec::with_capacity(entities.len() + scenes.len() + 1);
    let mut grouped = vec![false; entities.len()];

    for scene in scenes {
        let members: Vec<usize> = entities
            .iter()
            .enumerate()
            .filter(|(_, info)| info.scene == Some(scene.id))
            .map(|(idx, _)| idx)
            .collect();
        // Marked before the open check: a collapsed scene's entities are
        // still that scene's, and counting them as unsaved would move
        // them into another group the moment the group was closed.
        for &idx in &members {
            grouped[idx] = true;
        }

        let header = GroupHeader::scene(scene, members.len());
        let open = header.is_open(ui);
        rows.push(WorldRow::Group(header));
        if !open {
            continue;
        }
        if members.is_empty() {
            rows.push(WorldRow::Note("(empty)".to_owned()));
        }
        rows.extend(members.into_iter().map(WorldRow::Entity));
    }

    // Anything belonging to no scene still has to be reachable, or an
    // entity spawned before the first save would vanish from the panel
    // that is supposed to list the world.
    let orphans: Vec<usize> = grouped
        .iter()
        .enumerate()
        .filter(|&(_, done)| !done)
        .map(|(idx, _)| idx)
        .collect();
    if !orphans.is_empty() {
        let header = GroupHeader::unsaved(orphans.len());
        let open = header.is_open(ui);
        rows.push(WorldRow::Group(header));
        if open {
            rows.push(WorldRow::Note(
                "Not in any scene yet — saved with the active one.".to_owned(),
            ));
            rows.extend(orphans.into_iter().map(WorldRow::Entity));
        }
    }

    rows
}

/// Draws a group's header as one row of the list.
///
/// Hand-drawn rather than an [`egui::CollapsingHeader`]: that widget
/// draws its children inside its own closure, which is the one thing a
/// virtualized list cannot allow — the rows have to be siblings for the
/// scroll area to place them from an index. What it costs is this
/// function; what it buys is that a collapsed group's six hundred
/// entities are absent from the row list entirely rather than skipped
/// one at a time.
fn draw_group_header(ui: &mut egui::Ui, header: &GroupHeader, row_h: f32) {
    let size = egui::vec2(ui.available_width(), row_h - ui.spacing().item_spacing.y);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let mut open = header.is_open(ui);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|data| data.insert_persisted(header.id, open));
    }

    if !ui.is_rect_visible(rect) {
        return;
    }
    let visuals = *ui.style().interact(&resp);
    let icon_width = ui.spacing().icon_width;
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + icon_width * 0.5, rect.center().y),
        egui::vec2(icon_width, icon_width),
    );
    let mut icon_resp = resp.clone();
    icon_resp.rect = icon_rect;
    egui::collapsing_header::paint_default_icon(ui, if open { 1.0 } else { 0.0 }, &icon_resp);
    ui.painter().text(
        egui::pos2(
            rect.left() + icon_width + ui.spacing().item_spacing.x,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        &header.label,
        egui::TextStyle::Button.resolve(ui.style()),
        visuals.text_color(),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EntityDisplayInfo;

    fn entity_info(index: u32, scene: Option<kooch_core::Guid>) -> EntityDisplayInfo {
        EntityDisplayInfo {
            is_prefab_instance: false,
            entity: Entity::new(index, 0),
            components: Vec::new(),
            parent: None,
            children: Vec::new(),
            depth: 0,
            global_rotation: None,
            scene,
            parent_global_rotation: None,
        }
    }

    fn scene_info(id: kooch_core::Guid, active: bool) -> SceneDisplayInfo {
        SceneDisplayInfo {
            id,
            name: "Scene".to_owned(),
            dirty: false,
            active,
        }
    }

    /// Runs `body` against a real `Ui`, since everything here reads or
    /// writes egui's own layout and persisted state.
    fn with_ui<R>(body: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let ctx = egui::Context::default();
        let mut body = Some(body);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 600.0),
            )),
            ..Default::default()
        };
        let mut out = None;
        ctx.run_ui(input, |ui| {
            let body = body.take().expect("run_ui called the closure twice");
            egui::CentralPanel::default().show(ui, |ui| out = Some(body(ui)));
        });
        out.expect("central panel did not run")
    }

    #[test]
    fn one_scene_lists_every_entity_and_no_headers() {
        let entities: Vec<_> = (0..1000).map(|i| entity_info(i, None)).collect();
        let rows = with_ui(|ui| build_rows(ui, &entities, &[]));
        assert_eq!(rows.len(), 1000);
        assert!(rows.iter().all(|row| matches!(row, WorldRow::Entity(_))));
    }

    /// The point of building the list from the open flags: a collapsed
    /// group's entities are *absent*, not skipped. Skipping them one at a
    /// time would leave the cost proportional to the whole world, which
    /// is what collapsing is supposed to avoid.
    #[test]
    fn a_collapsed_group_contributes_only_its_header() {
        let a = kooch_core::Guid::new_v4();
        let b = kooch_core::Guid::new_v4();
        let entities: Vec<_> = (0..100)
            .map(|i| entity_info(i, Some(if i < 60 { a } else { b })))
            .collect();
        // Only `b` is active, so `a` defaults closed.
        let scenes = vec![scene_info(a, false), scene_info(b, true)];

        let rows = with_ui(|ui| build_rows(ui, &entities, &scenes));

        let headers = rows
            .iter()
            .filter(|row| matches!(row, WorldRow::Group(_)))
            .count();
        assert_eq!(headers, 2);
        assert_eq!(
            rows.len(),
            2 + 40,
            "the closed scene's 60 entities are still in the list",
        );
    }

    /// An entity in a closed scene belongs to that scene, not to nobody.
    /// Deciding group membership after the open check would move it into
    /// "Unsaved" as a side effect of clicking a triangle.
    #[test]
    fn a_collapsed_scenes_entities_do_not_become_unsaved() {
        let a = kooch_core::Guid::new_v4();
        let b = kooch_core::Guid::new_v4();
        let entities: Vec<_> = (0..10)
            .map(|i| entity_info(i, Some(if i < 5 { a } else { b })))
            .collect();
        let scenes = vec![scene_info(a, false), scene_info(b, true)];

        let rows = with_ui(|ui| build_rows(ui, &entities, &scenes));
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, WorldRow::Group(_)))
                .count(),
            2,
            "an Unsaved group appeared for entities that have a scene",
        );
    }

    #[test]
    fn an_entity_in_no_scene_is_still_reachable() {
        let a = kooch_core::Guid::new_v4();
        let b = kooch_core::Guid::new_v4();
        let mut entities: Vec<_> = (0..4).map(|i| entity_info(i, Some(a))).collect();
        entities.push(entity_info(99, None));
        let scenes = vec![scene_info(a, true), scene_info(b, true)];

        let rows = with_ui(|ui| build_rows(ui, &entities, &scenes));
        assert!(
            rows.iter()
                .any(|row| matches!(row, WorldRow::Entity(idx) if *idx == 4)),
            "the orphan entity is not in the list",
        );
    }

    /// The one invariant virtualization rests on. `show_rows` places every
    /// row from an index times this height without drawing the rows above,
    /// so a row that occupies anything else puts the whole list out of
    /// step with the scrollbar — and clicks land on a neighbour.
    #[test]
    fn a_row_occupies_exactly_the_height_the_list_reserved() {
        let entities = vec![entity_info(0, None)];
        let (reserved, occupied) = with_ui(|ui| {
            let reserved = entity_row::row_height(ui);
            let before = ui.cursor().top();
            draw_entity_row(
                ui,
                0,
                &entities[0],
                &entities,
                &mut Vec::new(),
                &mut std::collections::HashSet::new(),
                &[],
                &mut Vec::new(),
                &mut None,
            );
            (reserved, ui.cursor().top() - before)
        });
        assert!(
            (reserved - occupied).abs() < 0.01,
            "row reserved {reserved} but occupied {occupied}",
        );
    }

    /// The reason rows truncate. A name long enough to wrap would make
    /// its own row taller than the list promised, and every row below it
    /// would be drawn a line further off than the one before.
    #[test]
    fn a_very_long_name_does_not_make_its_row_taller() {
        let mut long = entity_info(0, None);
        long.depth = 4;
        long.components = vec![crate::state::ComponentDisplayInfo {
            type_id: std::any::TypeId::of::<()>(),
            component: kooch_ecs::component::ComponentId::INVALID,
            short_name: "Name".to_owned(),
            fields: Some(vec![(
                "value".to_owned(),
                kooch_ecs::reflect::ReflectValue::String("x".repeat(400)),
            )]),
            field_metas: None,
            visibility: Default::default(),
        }];
        let entities = vec![long];

        let (reserved, occupied) = with_ui(|ui| {
            let reserved = entity_row::row_height(ui);
            let before = ui.cursor().top();
            draw_entity_row(
                ui,
                0,
                &entities[0],
                &entities,
                &mut Vec::new(),
                &mut std::collections::HashSet::new(),
                &[],
                &mut Vec::new(),
                &mut None,
            );
            (reserved, ui.cursor().top() - before)
        });
        assert!(
            (reserved - occupied).abs() < 0.01,
            "a 400-character name made its row {occupied} tall, not {reserved}",
        );
    }
}
