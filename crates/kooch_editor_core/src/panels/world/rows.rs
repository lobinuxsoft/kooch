//! The hierarchy as rows: scene groups, members, scrolling to what was just picked.

use super::*;

/// One line of the hierarchy, as the virtualized list addresses it.
pub(super) enum WorldRow {
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
pub(super) struct GroupHeader {
    /// What identifies the group across frames. Scenes use their id; the
    /// unsaved group has no scene to name it, hence the string.
    pub(super) id: egui::Id,
    pub(super) label: String,
    /// Whether it starts open the first time it is ever seen.
    pub(super) default_open: bool,
    /// The scene this header stands for, or `None` for the pseudo-group holding entities that
    /// belong to none.
    pub(super) scene: Option<kooch_core::Guid>,
    /// Whether that scene has edits not on disk.
    pub(super) dirty: bool,
    /// Whether it has ever been saved.
    pub(super) has_file: bool,
    /// Whether new entities land here.
    pub(super) active: bool,
}

impl GroupHeader {
    pub(super) fn scene(scene: &SceneDisplayInfo, count: usize) -> Self {
        // Leading, not trailing. The entity count sits between the name and the end of the line, so
        // an asterisk after it is separated from the thing it is about by a number that changes —
        // and in a column of scenes, the eye scans the left edge.
        let dirty = if scene.dirty { "*" } else { "" };
        Self {
            id: egui::Id::new(("world_group_open", scene.id)),
            label: format!("{dirty}{} ({count} entities)", scene.name),
            // The active scene starts expanded: it is the one being
            // worked in.
            default_open: scene.active,
            scene: Some(scene.id),
            dirty: scene.dirty,
            has_file: scene.path.is_some(),
            active: scene.active,
        }
    }

    /// A scene's header while the panel is filtered.
    pub(super) fn filtered(scene: &SceneDisplayInfo, shown: usize, total: usize) -> Self {
        let dirty = if scene.dirty { "*" } else { "" };
        Self {
            id: egui::Id::new(("world_group_filtered", scene.id)),
            label: format!("{dirty}{} ({shown} of {total})", scene.name),
            default_open: true,
            scene: Some(scene.id),
            dirty: scene.dirty,
            has_file: scene.path.is_some(),
            active: scene.active,
        }
    }

    /// The unsaved pseudo-group's header while filtered.
    pub(super) fn unsaved_filtered(shown: usize, total: usize) -> Self {
        Self {
            id: egui::Id::new("world_group_filtered_unsaved"),
            label: format!("Unsaved ({shown} of {total})"),
            default_open: true,
            scene: None,
            dirty: false,
            has_file: false,
            active: false,
        }
    }

    pub(super) fn unsaved(count: usize) -> Self {
        Self {
            id: egui::Id::new("world_group_open_unsaved"),
            label: format!("Unsaved ({count} entities)"),
            default_open: true,
            scene: None,
            dirty: false,
            has_file: false,
            active: false,
        }
    }

    /// Whether the group is open, remembered across frames.
    pub(super) fn is_open(&self, ui: &egui::Ui) -> bool {
        ui.data_mut(|data| *data.get_persisted_mut_or_insert_with(self.id, || self.default_open))
    }

    /// Opens the group. Used when a selection lands inside a closed one: a row that does not exist
    /// cannot be scrolled to, and leaving it closed reproduces the very symptom this is fixing —
    /// something selected with nothing on screen to show for it (#706).
    pub(super) fn open(&self, ui: &egui::Ui) {
        ui.data_mut(|data| data.insert_persisted(self.id, true));
    }
}

/// Where the last drawn row range is kept, so the writer and the reader cannot drift apart by a
/// typo. They already did once: a refactor took the write with it and left the read, which every
/// test passed because each one wrote the value itself.
pub(super) fn visible_range_id() -> egui::Id {
    egui::Id::new("world_visible_range")
}

/// The entity the selection just moved to, or `None` if it did not move.
pub(super) fn newly_focused(ui: &egui::Ui, selected: &[Entity]) -> Option<Entity> {
    let id = egui::Id::new("world_scrolled_to");
    // The most recently added, not the first: with several selected, the
    // one worth showing is the one that just happened.
    let focus = selected.last().copied();
    let previous = ui.data(|d| d.get_temp::<Option<Entity>>(id).flatten());
    if focus == previous {
        return None;
    }
    ui.data_mut(|d| d.insert_temp(id, focus));
    focus
}

/// Where the list has to be scrolled to put `focus` on screen, or `None` if it is already there.
pub(super) fn scroll_offset_for(
    ui: &egui::Ui,
    rows: &[WorldRow],
    entities: &[EntityDisplayInfo],
    focus: Entity,
    row_h: f32,
) -> Option<f32> {
    // What `show_rows` actually divides a scroll offset by. The row height it is handed is *sans
    // spacing* — `scroll_area.rs` adds `item_spacing.y` before mapping an offset to a row index.
    let pitch = row_pitch(ui, row_h);
    let index = rows.iter().position(|row| match row {
        WorldRow::Entity(idx) => entities.get(*idx).is_some_and(|e| e.entity == focus),
        _ => false,
    })?;

    // Already on screen — scrolling would yank the list for a row that needed nothing, which is
    // what clicking a visible row would feel like. The range is last frame's; one frame of
    // staleness is worth less than the alternative of guessing.
    let visible = ui.data(|d| d.get_temp::<(usize, usize)>(visible_range_id()));
    if let Some((start, end)) = visible
        && (start..end).contains(&index)
    {
        return None;
    }

    // Centred rather than pinned to the top: a row at the very edge of
    // the viewport reads as "the list happens to end here", not as "this
    // is the thing you just picked".
    let viewport_rows = visible.map_or(0.0, |(start, end)| (end - start) as f32);
    let centre = (viewport_rows / 2.0 - 0.5).max(0.0);
    Some(((index as f32 - centre) * pitch).max(0.0))
}

/// The distance from one row's top to the next, which is what a scroll offset is measured in.
pub(super) fn row_pitch(ui: &egui::Ui, row_h: f32) -> f32 {
    row_h + ui.spacing().item_spacing.y
}

/// Opens whatever group and collapsed ancestors hold `entity`, so it has a row to scroll to.
pub(super) fn reveal_group_of(
    ui: &egui::Ui,
    entities: &[EntityDisplayInfo],
    scenes: &[SceneDisplayInfo],
    entity: Entity,
) {
    // 🔴 No early return on a single scene. That guard was right while a lone scene drew no header,
    // and stopped being right the moment every scene got a root — one collapsed scene is now
    // exactly as able to hide a selection as two.
    let Some(info) = entities.iter().find(|e| e.entity == entity) else {
        return;
    };

    // Every collapsed ancestor, not only the group. A prefab instance
    // starts collapsed, so anything created inside one — a duplicate of a
    // child — lands in a subtree with no rows at all.
    let mut ancestor = info.parent;
    while let Some(parent) = ancestor {
        ui.data_mut(|data| data.insert_persisted(subtree_id(parent), true));
        ancestor = entities
            .iter()
            .find(|e| e.entity == parent)
            .and_then(|e| e.parent);
    }

    match info.scene.and_then(|id| scenes.iter().find(|s| s.id == id)) {
        Some(scene) => {
            let members = entities
                .iter()
                .filter(|e| e.scene == Some(scene.id))
                .count();
            GroupHeader::scene(scene, members).open(ui);
        }
        // Belongs to no scene: it lives under the unsaved group.
        None => {
            let orphans = entities.iter().filter(|e| e.scene.is_none()).count();
            GroupHeader::unsaved(orphans).open(ui);
        }
    }
}

/// Flattens the hierarchy into the lines the panel will show.
/// What identifies one entity's expanded state across frames.
pub(super) fn subtree_id(entity: Entity) -> egui::Id {
    egui::Id::new(("world_subtree_open", entity))
}

/// Whether an entity's children are listed under it.
pub(super) fn subtree_open(ui: &egui::Ui, entity: Entity, default_open: bool) -> bool {
    ui.data_mut(|data| *data.get_persisted_mut_or_insert_with(subtree_id(entity), || default_open))
}

/// Appends a scene's entities, leaving out what a collapsed parent hides.
pub(super) fn push_members(
    ui: &egui::Ui,
    entities: &[EntityDisplayInfo],
    members: &[usize],
    rows: &mut Vec<WorldRow>,
) {
    // The depth of the collapsed parent whose descendants are being
    // dropped, if any.
    let mut hidden_under: Option<usize> = None;
    // Whether each level of the current chain sits inside a prefab
    // instance, so the instance's ROOT can be told from its members —
    // `is_prefab_instance` is true for every entity the instance owns.
    let mut inside_prefab: Vec<bool> = Vec::new();

    for &idx in members {
        let Some(info) = entities.get(idx) else {
            continue;
        };
        match hidden_under {
            Some(depth) if info.depth > depth => continue,
            _ => hidden_under = None,
        }

        inside_prefab.truncate(info.depth);
        let under_instance = inside_prefab.last().copied().unwrap_or(false);
        inside_prefab.push(under_instance || info.is_prefab_instance);

        rows.push(WorldRow::Entity(idx));

        if info.children.is_empty() {
            continue;
        }
        let starts_open = !(info.is_prefab_instance && !under_instance);
        if !subtree_open(ui, info.entity, starts_open) {
            hidden_under = Some(info.depth);
        }
    }
}

pub(super) fn build_rows(
    ui: &egui::Ui,
    entities: &[EntityDisplayInfo],
    scenes: &[SceneDisplayInfo],
    filter: &WorldFilter,
) -> Vec<WorldRow> {
    let mut rows = Vec::with_capacity(entities.len() + scenes.len() + 1);
    let mut grouped = vec![false; entities.len()];

    // 🔴 A header even for a single scene. It used to be skipped — "every row would sit under the
    // same one" — and that was true right up until a second scene could be opened beside it.
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

        if filter.active() {
            let matched: Vec<usize> = members
                .iter()
                .copied()
                .filter(|&idx| entities.get(idx).is_some_and(|info| filter.matches(info)))
                .collect();
            // A header over nothing is a row that says "not here" in the
            // most expensive way available. Skipped entirely.
            if matched.is_empty() {
                continue;
            }
            rows.push(WorldRow::Group(GroupHeader::filtered(
                scene,
                matched.len(),
                members.len(),
            )));
            // 🔴 Flat, and the collapse state is ignored. A match hidden under a closed parent is a
            // search that found the thing and did not show it, which is worse than finding nothing.
            // The rows keep their own indent, so each one still says where it lives.
            rows.extend(matched.into_iter().map(WorldRow::Entity));
            continue;
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
        push_members(ui, entities, &members, &mut rows);
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
        if filter.active() {
            let matched: Vec<usize> = orphans
                .iter()
                .copied()
                .filter(|&idx| entities.get(idx).is_some_and(|info| filter.matches(info)))
                .collect();
            if !matched.is_empty() {
                rows.push(WorldRow::Group(GroupHeader::unsaved_filtered(
                    matched.len(),
                    orphans.len(),
                )));
                rows.extend(matched.into_iter().map(WorldRow::Entity));
            }
        } else {
            let header = GroupHeader::unsaved(orphans.len());
            let open = header.is_open(ui);
            rows.push(WorldRow::Group(header));
            if open {
                rows.push(WorldRow::Note(
                    "Not in any scene yet — saved with the active one.".to_owned(),
                ));
                push_members(ui, entities, &orphans, &mut rows);
            }
        }
    }

    // Says so, rather than showing an empty panel that is
    // indistinguishable from an empty world.
    if filter.active() && rows.is_empty() {
        rows.push(WorldRow::Note("No entity matches the filter.".to_owned()));
    }

    rows
}

/// Draws a group's header as one row of the list.
pub(super) fn draw_group_header(
    ui: &mut egui::Ui,
    header: &GroupHeader,
    row_h: f32,
    clipboard_has_entities: bool,
    selected: &[Entity],
    actions: &mut Vec<EditorAction>,
) {
    let size = egui::vec2(ui.available_width(), row_h);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let mut open = header.is_open(ui);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|data| data.insert_persisted(header.id, open));
    }
    scene_context_menu(&resp, header, clipboard_has_entities, actions);
    // Dropping a row here re-homes it. The direct-manipulation form of
    // the menu's Paste, and a MOVE: an entity belongs to exactly one
    // scene, so the one it came from stops holding it.
    let dropped = header
        .scene
        .and_then(|scene| resp.dnd_release_payload::<Entity>().map(|e| (scene, *e)));
    if let Some((scene, dragged)) = dropped {
        // The whole selection when the row being dragged is part of it:
        // selecting six and dragging one of them means the six, which is
        // what every other panel that drags does.
        let moving: Vec<Entity> = match selected.contains(&dragged) {
            true => selected.to_vec(),
            false => vec![dragged],
        };
        for entity in moving {
            actions.push(EditorAction::MoveToScene { entity, scene });
        }
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
        match header.dirty {
            true => DIRTY_SCENE,
            false => visuals.text_color(),
        },
    );

    // Over the row rather than under it, so the header still reads
    // through the tint. Only for a group that is a scene: "Unsaved" is
    // not a place an entity can be moved TO.
    if header.scene.is_some() && resp.dnd_hover_payload::<Entity>().is_some() {
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(60, 200, 100, 40),
        );
    }
}
