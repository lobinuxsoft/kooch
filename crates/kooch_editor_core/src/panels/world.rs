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

    // Read, drawn, written back. Held in egui's temp store rather than
    // threaded through the editor's state for the same reason the group
    // flags are: it is view state of one panel and it should not outlive
    // the session. See `WorldFilter`.
    let filter_id = egui::Id::new("world_filter");
    let mut filter = ui.data_mut(|d| d.get_temp::<WorldFilter>(filter_id).unwrap_or_default());
    draw_filter_bar(ui, entities, &mut filter);
    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
    ui.separator();

    // Every line the panel will show, headers included, before any of
    // them is drawn. Collapsed scenes contribute their header and
    // nothing else, so the list is exactly what is on screen and its
    // length is exactly what the scrollbar should describe.
    // Asked before the rows are built: opening a group changes what the
    // list contains, and the index the scroll is computed from has to be
    // an index into the list that will actually be drawn (#706).
    let focus = newly_focused(ui, selected);
    if let Some(focus) = focus {
        reveal_group_of(ui, entities, scenes, focus);
    }

    let rows = build_rows(ui, entities, scenes, &filter);
    // 🔴 What the panel is actually SHOWING, in the order it shows it —
    // the filter applied, collapsed subtrees left out. Every gesture that
    // spans more than one row reads this instead of the display list:
    // Shift+Click, Ctrl+A and the arrows all used to walk `entities`, so
    // a range between two visible rows swept up every hidden entity
    // between them and Ctrl+A selected two thousand while four were on
    // screen.
    let listed: Vec<usize> = rows
        .iter()
        .filter_map(|row| match row {
            WorldRow::Entity(idx) => Some(*idx),
            _ => None,
        })
        .collect();
    // ⚠️ After the rows, so it can be told what is listed — which means a
    // selection moved by the keyboard is revealed and scrolled to on the
    // NEXT frame rather than this one. One frame, and only for the
    // arrows: a click is handled inside the row, where the rows already
    // exist.
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

    // 🔴 Claimed BEFORE the list, over the whole panel, and that order
    // is the fix rather than a detail. The target used to be allocated
    // INSIDE the virtualized closure, out of what was left under the
    // last row — which meant it existed only once the list had drawn its
    // final row (never, in a scene of two thousand, unless scrolled to
    // the bottom) and measured zero high when the groups were collapsed,
    // because inside `show_rows` `max_rect` is the virtualized CONTENT,
    // not the panel. Either way there was nothing under the pointer and
    // the right click landed on a background that offered nothing.
    //
    // Claimed first, every row drawn afterwards sits on top of it and
    // wins the overlap, so this answers exactly where no row is.
    let background_rect = ui.available_rect_before_wrap();
    let background = ui.interact(
        background_rect,
        ui.id().with("world_background"),
        egui::Sense::click(),
    );

    let mut area = egui::ScrollArea::vertical()
        .id_salt("world_tree")
        // Fills the panel instead of shrinking to whatever is drawn. A
        // virtualized list only ever draws the rows that fit, so letting
        // it shrink leaves a gap under the last one and puts the drop
        // target for "unparent" somewhere other than the bottom of the
        // panel.
        .auto_shrink([false; 2]);
    if let Some(offset) = scroll_to {
        // Applied on this frame only. Setting it every frame would pin
        // the list in place and there would be no scrolling by hand.
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, row_h, rows.len(), |ui, range| {
        // Recorded so the next selection can tell whether its row is
        // already on screen. Written here because this is the only place
        // that knows it — and read by `scroll_offset_for`, which is what
        // stops a click on a visible row from yanking the list.
        ui.data_mut(|d| {
            d.insert_temp(visible_range_id(), (range.start, range.end));
        });

        // The range is the slice of rows that fits on screen — twenty
        // of them, whether the scene holds six hundred entities or
        // sixty thousand. Everything a row costs (laying out its
        // text, sensing clicks, registering a drop target, walking
        // the parent chain to reject a cyclic reparent) is paid for
        // the rows a person can see rather than for the ones the
        // scroll position happens to be nowhere near.
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

    // The same entries the toolbar's Spawn button offers, reached
    // where people actually reach for them: right-click in the
    // empty part of the hierarchy. A row's own right-click menu
    // handles per-entity actions, including Add Component (#591).
    //
    // 🔴 Into a scene of its own, not the active one. Right-clicking
    // past the last row is not "put this somewhere" — there is no
    // row under the pointer to name a somewhere. It is "start
    // something new", and since an entity has to belong to a scene,
    // starting one is what makes the gesture answerable.
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
    // A prefab dropped into the hierarchy spawns at the position it
    // was authored at: a list of names has no geometry to read a
    // place out of, and defaulting to the origin would silently move
    // a prefab that was deliberately authored elsewhere. Drop it in
    // the View panel to choose a spot.
    // Filtered by type the way an Inspector asset slot is: a mesh
    // dragged over the hierarchy is not something to instance.
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
    /// The scene this header stands for, or `None` for the pseudo-group
    /// holding entities that belong to none.
    ///
    /// What decides whether the row has anything to offer on a right
    /// click: "Save" means nothing for a group that is not a file.
    scene: Option<kooch_core::Guid>,
    /// Whether that scene has edits not on disk.
    dirty: bool,
    /// Whether it has ever been saved.
    ///
    /// What separates "discard changes" from "delete everything": a scene
    /// with no file has nothing to be read back from.
    has_file: bool,
    /// Whether new entities land here.
    ///
    /// Carried so the header's own menu can say so and change it. With
    /// one scene open `draw_scene_bar` hides itself, and this row is then
    /// the only thing on screen that names the scene at all.
    active: bool,
}

impl GroupHeader {
    fn scene(scene: &SceneDisplayInfo, count: usize) -> Self {
        // Leading, not trailing. The entity count sits between the name
        // and the end of the line, so an asterisk after it is separated
        // from the thing it is about by a number that changes — and in a
        // column of scenes, the eye scans the left edge.
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
    ///
    /// 🔴 Always open, and it says `shown of total`. A collapsible group
    /// under a filter is a second way to hide a row somebody is looking
    /// for, and a bare count would read as the scene having shrunk.
    fn filtered(scene: &SceneDisplayInfo, shown: usize, total: usize) -> Self {
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
    fn unsaved_filtered(shown: usize, total: usize) -> Self {
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

    fn unsaved(count: usize) -> Self {
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
    ///
    /// Kept in egui's own persisted store rather than in the editor's
    /// state: it is view state of one panel, it should survive a frame
    /// and not a project, and threading it through would put a field
    /// about a scrollbar into the editor's model of the world.
    fn is_open(&self, ui: &egui::Ui) -> bool {
        ui.data_mut(|data| *data.get_persisted_mut_or_insert_with(self.id, || self.default_open))
    }

    /// Opens the group. Used when a selection lands inside a closed one:
    /// a row that does not exist cannot be scrolled to, and leaving it
    /// closed reproduces the very symptom this is fixing — something
    /// selected with nothing on screen to show for it (#706).
    fn open(&self, ui: &egui::Ui) {
        ui.data_mut(|data| data.insert_persisted(self.id, true));
    }
}

/// Where the last drawn row range is kept, so the writer and the reader
/// cannot drift apart by a typo. They already did once: a refactor took
/// the write with it and left the read, which every test passed because
/// each one wrote the value itself.
fn visible_range_id() -> egui::Id {
    egui::Id::new("world_visible_range")
}

/// The entity the selection just moved to, or `None` if it did not move.
///
/// Consumes the change: called once per frame, it answers `Some` exactly
/// on the frame the selection became something new. That matters for
/// more than efficiency — everything downstream *acts*, and acting every
/// frame would mean a group reopening itself the instant you closed it
/// with something selected inside.
fn newly_focused(ui: &egui::Ui, selected: &[Entity]) -> Option<Entity> {
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

/// Where the list has to be scrolled to put `focus` on screen, or `None`
/// if it is already there.
///
/// # Why this is arithmetic and not a widget call
///
/// egui's `scroll_to_me` needs the row to exist, and in a virtualized
/// list the selected row is usually not drawn at all: `show_rows` builds
/// the twenty that fit and no others. But it does not need to exist. The
/// list `build_rows` returns is the complete one, and `row_height` is the
/// single definition of how tall a line is — the two facts virtualization
/// itself rests on. So the position of any row is `index * row_height`,
/// whether or not anybody drew it.
///
/// Which is also why opening a collapsed group costs nothing here: the
/// group's entities join the row list, the list gets longer, and the same
/// multiplication answers the same question.
fn scroll_offset_for(
    ui: &egui::Ui,
    rows: &[WorldRow],
    entities: &[EntityDisplayInfo],
    focus: Entity,
    row_h: f32,
) -> Option<f32> {
    // What `show_rows` actually divides a scroll offset by. The row
    // height it is handed is *sans spacing* — `scroll_area.rs` adds
    // `item_spacing.y` before mapping an offset to a row index. Using
    // the bare height here loses a few pixels per row, which nobody
    // notices at row ten and lands sixty-eight rows off at row 460.
    let pitch = row_pitch(ui, row_h);
    let index = rows.iter().position(|row| match row {
        WorldRow::Entity(idx) => entities.get(*idx).is_some_and(|e| e.entity == focus),
        _ => false,
    })?;

    // Already on screen — scrolling would yank the list for a row that
    // needed nothing, which is what clicking a visible row would feel
    // like. The range is last frame's; one frame of staleness is worth
    // less than the alternative of guessing.
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

/// The distance from one row's top to the next, which is what a scroll
/// offset is measured in.
///
/// [`entity_row::row_height`] is the height of a row's contents;
/// `ScrollArea::show_rows` adds `item_spacing.y` on top of it. Both
/// numbers are needed and they are not the same one — which is the sort
/// of difference that reads as correct on screen until the list is long.
fn row_pitch(ui: &egui::Ui, row_h: f32) -> f32 {
    row_h + ui.spacing().item_spacing.y
}

/// Opens whatever group and collapsed ancestors hold `entity`, so it has
/// a row to scroll to.
///
/// A row that does not exist cannot be scrolled to, and leaving it hidden
/// reproduces the symptom #706 exists to prevent: something selected with
/// nothing on screen to show for it.
fn reveal_group_of(
    ui: &egui::Ui,
    entities: &[EntityDisplayInfo],
    scenes: &[SceneDisplayInfo],
    entity: Entity,
) {
    // 🔴 No early return on a single scene. That guard was right while a
    // lone scene drew no header, and stopped being right the moment every
    // scene got a root — one collapsed scene is now exactly as able to
    // hide a selection as two.
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
///
/// 🔴 A prefab instance starts CLOSED and everything else starts open,
/// and that is the difference between a panel and a wall. An instance is
/// a unit: its five entities are the prefab's business, not the scene's,
/// and `many_lights` puts thirty-six of them on screen — a hundred and
/// eighty rows nobody asked to read. A hand-built hierarchy is the
/// opposite: somebody put those children there on purpose, and hiding
/// them would hide their own work.
///
/// Kept in egui's persisted store like the group headers, for the same
/// reason: it is view state of one panel, it should survive a frame and
/// not a project, and threading it through would put a field about a
/// disclosure triangle into the editor's model of the world.
fn subtree_open(ui: &egui::Ui, entity: Entity, default_open: bool) -> bool {
    ui.data_mut(|data| *data.get_persisted_mut_or_insert_with(subtree_id(entity), || default_open))
}

/// Appends a scene's entities, leaving out what a collapsed parent hides.
///
/// `members` arrive in DFS order with `depth` — the third pass of
/// `queries` guarantees it — so a subtree is contiguous and skipping one
/// is "drop rows until the depth comes back up". That is also why a
/// collapsed group costs nothing: its rows are absent from the list
/// rather than skipped one at a time while scrolling.
fn push_members(
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

fn build_rows(
    ui: &egui::Ui,
    entities: &[EntityDisplayInfo],
    scenes: &[SceneDisplayInfo],
    filter: &WorldFilter,
) -> Vec<WorldRow> {
    let mut rows = Vec::with_capacity(entities.len() + scenes.len() + 1);
    let mut grouped = vec![false; entities.len()];

    // 🔴 A header even for a single scene. It used to be skipped —
    // "every row would sit under the same one" — and that was true right
    // up until a second scene could be opened beside it. Without a root
    // per scene, two scenes' entities land in one column with nothing
    // saying which is which, no place to offer closing one, and no
    // answer to which scene a Spawn belongs to. The root is what makes
    // additive open a thing that can exist.
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
            // 🔴 Flat, and the collapse state is ignored. A match hidden
            // under a closed parent is a search that found the thing and
            // did not show it, which is worse than finding nothing. The
            // rows keep their own indent, so each one still says where
            // it lives.
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
///
/// Hand-drawn rather than an [`egui::CollapsingHeader`]: that widget
/// draws its children inside its own closure, which is the one thing a
/// virtualized list cannot allow — the rows have to be siblings for the
/// scroll area to place them from an index. What it costs is this
/// function; what it buys is that a collapsed group's six hundred
/// entities are absent from the row list entirely rather than skipped
/// one at a time.
fn draw_group_header(
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

/// The colour of a scene that has edits not on disk.
///
/// The same amber the code-sync control pulses in, because it means the
/// same thing: something here is out of step with what is on disk. One
/// colour for "needs attention" is one thing to learn rather than two.
///
/// Paired with the `*`, never alone — a colour by itself is unreadable
/// to anyone who cannot separate these two hues, and this one has to
/// survive both themes.
///
/// 🔴 A literal, and it should not stay one. Every colour the panel
/// introduces belongs in Settings (#955); a palette hard-coded into a
/// panel is a palette nobody can fix for their own eyes.
const DIRTY_SCENE: egui::Color32 = egui::Color32::from_rgb(210, 150, 60);

/// The right-click menu on a scene's row.
///
/// Saving one scene at a time is what having several open makes
/// necessary: the File menu saves the active scene, and the scene
/// somebody right-clicked is routinely not that one. Writing the wrong
/// file is not a mistake the user can see until the next load.
///
/// Nothing for the "Unsaved" pseudo-group — it is not a file, so there
/// is nowhere for it to be saved to. Its entities go with the active
/// scene, which is what its own note already says.
fn scene_context_menu(
    resp: &egui::Response,
    header: &GroupHeader,
    clipboard_has_entities: bool,
    actions: &mut Vec<EditorAction>,
) {
    let Some(scene) = header.scene else {
        return;
    };
    resp.context_menu(|ui| {
        ui.set_min_width(240.0);
        // 🔴 The only place the active scene can be chosen when a single
        // scene is open: `draw_scene_bar` hides itself under two scenes,
        // so with one there was nothing on screen naming it and nothing
        // to click. "I cannot select the scene" is a correct reading of
        // a panel that never offered.
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
        // No icon on either. There is no verified Phosphor codepoint for
        // a save glyph in `icons`, and that module's own note says why
        // guessing one is not an option: a wrong codepoint is still a
        // valid glyph, so it renders something and only a person looking
        // at it ever finds out. Eleven of the first thirty were wrong.
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
        // Only offered when there is something to discard, and only for
        // a scene that has a file. Without one there is nothing to revert
        // *to*, and despawning its entities would delete work rather than
        // undo it — the one thing "discard" must never be mistaken for.
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
        // 🔴 The only way to close ONE scene without the scene bar, which
        // hides itself under two scenes — so the moment additive opening
        // gave you a second scene, closing either one had no gesture but
        // that bar. Right-clicking the scene you want gone is the obvious
        // place to ask.
        //
        // No confirmation and no icon: `CloseScene` discards unsaved
        // edits, and the label says which scene by being ON it. The
        // hover text is where the warning goes, because a dialog for
        // every close is how people stop reading dialogs.
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

#[cfg(test)]
mod tests;
