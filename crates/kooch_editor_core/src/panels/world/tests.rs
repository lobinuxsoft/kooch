use super::filter::WorldFilter;
use super::*;
use crate::state::EntityDisplayInfo;
use crate::state::ReflectedFields;

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
        path: None,
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

/// A pick in the viewport has to bring its row into view, and the
/// row is usually not drawn — which is the whole difficulty (#706).
#[test]
fn a_selection_far_down_the_list_scrolls_into_view() {
    let entities: Vec<_> = (0..1000).map(|i| entity_info(i, None)).collect();
    let selected = vec![entities[900].entity];

    let offset = with_ui(|ui| {
        let rows = build_rows(ui, &entities, &[], &WorldFilter::default());
        // The list has been sitting at the top.
        ui.data_mut(|d| d.insert_temp(visible_range_id(), (0usize, 20usize)));
        let focus = newly_focused(ui, &selected).expect("selection is new");
        let row = rows
            .iter()
            .position(|row| matches!(row, WorldRow::Entity(idx) if entities[*idx].entity == focus))
            .expect("the focused entity has a row");
        scroll_offset_for(ui, &rows, &entities, focus, 20.0)
            .map(|offset| (offset, row, row_pitch(ui, 20.0)))
    });

    let (offset, row, pitch) = offset.expect("a row 900 places down is not on screen");
    // Centred on the row the entity actually landed on, with twenty
    // visible, measured in the pitch `show_rows` uses — height *plus*
    // item spacing.
    //
    // 🔴 The row index is read back rather than assumed to be 900. The
    // list carries group headers and notes now, so "the 900th entity" and
    // "the 900th row" are different numbers, and a test that hardcodes
    // one of them is measuring the layout instead of the scrolling.
    assert!(
        (offset - (row as f32 - 9.5) * pitch).abs() < 1.0,
        "expected row {row} centred, got offset {offset} at pitch {pitch}",
    );
}

/// The half of the exchange no other test covered: every test here
/// wrote the visible range itself, so deleting the line that records
/// it left them all green while the list scrolled on every click.
///
/// Which is what happened — a refactor took the write and left the
/// read, and the suite said nothing.
#[test]
fn the_panel_records_the_range_it_drew() {
    let mut entities: Vec<_> = (0..500).map(|i| entity_info(i, None)).collect();
    let mut selected = Vec::new();
    let mut pinned = std::collections::HashSet::new();
    let mut actions = Vec::new();
    let mut last_clicked = None;

    let recorded = with_ui(|ui| {
        ui.data_mut(|d| d.remove::<(usize, usize)>(visible_range_id()));
        draw_world_content(
            ui,
            true,
            &mut entities,
            &mut selected,
            &mut pinned,
            &[],
            &mut actions,
            500,
            1,
            1,
            &mut last_clicked,
            &[],
            false,
        );
        ui.data(|d| d.get_temp::<(usize, usize)>(visible_range_id()))
    });

    let (start, end) = recorded.expect("the panel has to say what it drew");
    assert!(end > start, "an empty range describes nothing");
    assert!(end <= 500, "the range cannot exceed the rows");
}

/// The offset has to be measured in the same unit `show_rows`
/// divides by, or it is right at row ten and sixty rows off at row
/// 460 — which is what shipped and what a person saw immediately.
///
/// Pinned against `egui`'s own arithmetic
/// (`scroll_area.rs`: `row_height_sans_spacing + spacing.y`) rather
/// than a number copied here, so an upstream change to it fails this
/// instead of silently drifting the list.
#[test]
fn the_offset_is_in_the_unit_show_rows_reads() {
    with_ui(|ui| {
        let row_h = entity_row::row_height(ui);
        let spacing = ui.spacing().item_spacing.y;
        assert!(spacing > 0.0, "a zero spacing would make this test vacuous");
        assert_eq!(row_pitch(ui, row_h), row_h + spacing);
    });
}

/// Clicking a row that is already visible must not move the list —
/// the selection changed, but nothing needed to happen.
#[test]
fn selecting_a_visible_row_leaves_the_list_alone() {
    let entities: Vec<_> = (0..1000).map(|i| entity_info(i, None)).collect();
    let selected = vec![entities[5].entity];

    let offset = with_ui(|ui| {
        let rows = build_rows(ui, &entities, &[], &WorldFilter::default());
        ui.data_mut(|d| d.insert_temp(visible_range_id(), (0usize, 20usize)));
        let focus = newly_focused(ui, &selected).expect("selection is new");
        scroll_offset_for(ui, &rows, &entities, focus, 20.0)
    });

    assert_eq!(offset, None, "row 5 of 0..20 is already on screen");
}

/// The reason change detection is separate from acting on it: a
/// group closed by hand, with something selected inside it, must
/// stay closed. Asking every frame would reopen it every frame.
#[test]
fn an_unchanged_selection_asks_for_nothing() {
    let entities: Vec<_> = (0..100).map(|i| entity_info(i, None)).collect();
    let selected = vec![entities[50].entity];

    with_ui(|ui| {
        assert!(newly_focused(ui, &selected).is_some(), "first sight of it");
        assert!(
            newly_focused(ui, &selected).is_none(),
            "the same selection is not news twice",
        );
    });
}

/// An entity inside a collapsed group has no row at all. Opening the
/// group gives it one — and the row list, longer now, still places
/// it by the same multiplication.
#[test]
fn a_selection_inside_a_collapsed_group_is_revealed() {
    let first = kooch_core::Guid::new_v4();
    let second = kooch_core::Guid::new_v4();
    let scenes = vec![scene_info(first, true), scene_info(second, false)];
    let entities: Vec<_> = (0..40)
        .map(|i| entity_info(i, Some(if i < 20 { first } else { second })))
        .collect();
    let hidden = entities[30].entity;

    with_ui(|ui| {
        // Close the second group: its twenty entities leave the list.
        GroupHeader::scene(&scenes[1], 20).open(ui);
        ui.data_mut(|d| d.insert_persisted(egui::Id::new(("world_group_open", second)), false));
        let closed = build_rows(ui, &entities, &scenes, &WorldFilter::default());
        assert!(
            !closed.iter().any(|row| matches!(row, WorldRow::Entity(idx)
                    if entities[*idx].entity == hidden)),
            "a closed group contributes no entity rows",
        );

        reveal_group_of(ui, &entities, &scenes, hidden);
        let opened = build_rows(ui, &entities, &scenes, &WorldFilter::default());
        let index = opened.iter().position(|row| {
            matches!(row, WorldRow::Entity(idx)
                if entities[*idx].entity == hidden)
        });
        assert!(index.is_some(), "revealing the group gave it a row");
    });
}

/// A single scene still gets a root of its own.
///
/// 🔴 It used to be skipped, on the argument that "every row would sit
/// under the same one". That was true right up until a second scene
/// could be opened beside it: without a root per scene, two scenes'
/// entities land in one column with nothing saying which is which, no
/// place to offer closing one, and no answer to which scene a Spawn
/// belongs to. The root is what makes additive open a thing that can
/// exist, so it is there even when there is only one.
#[test]
fn a_single_scene_still_gets_a_root() {
    let id = kooch_core::Guid::new_v4();
    let entities: Vec<_> = (0..1000).map(|i| entity_info(i, Some(id))).collect();
    let scenes = vec![scene_info(id, true)];
    let rows = with_ui(|ui| build_rows(ui, &entities, &scenes, &WorldFilter::default()));

    assert!(
        matches!(rows.first(), Some(WorldRow::Group(_))),
        "the scene has no root row"
    );
    assert_eq!(rows.len(), 1001, "one root plus every entity");
    assert!(
        rows[1..]
            .iter()
            .all(|row| matches!(row, WorldRow::Entity(_))),
        "something other than entities landed under the root"
    );
}

/// Entities belonging to no scene are still reachable, under their own
/// header — an entity spawned before the first save must not vanish
/// from the panel that lists the world.
#[test]
fn entities_without_a_scene_get_their_own_group() {
    let entities: Vec<_> = (0..10).map(|i| entity_info(i, None)).collect();
    let rows = with_ui(|ui| build_rows(ui, &entities, &[], &WorldFilter::default()));

    assert!(matches!(rows.first(), Some(WorldRow::Group(_))));
    let listed = rows
        .iter()
        .filter(|row| matches!(row, WorldRow::Entity(_)))
        .count();
    assert_eq!(listed, 10, "an orphan went missing");
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

    let rows = with_ui(|ui| build_rows(ui, &entities, &scenes, &WorldFilter::default()));

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

    let rows = with_ui(|ui| build_rows(ui, &entities, &scenes, &WorldFilter::default()));
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

    let rows = with_ui(|ui| build_rows(ui, &entities, &scenes, &WorldFilter::default()));
    assert!(
        rows.iter()
            .any(|row| matches!(row, WorldRow::Entity(idx) if *idx == 4)),
        "the orphan entity is not in the list",
    );
}

/// `show_rows` names its parameter `row_height_sans_spacing` and adds
/// `item_spacing.y` itself. A height that already includes it makes
/// egui reserve two gaps per row while each row leaves one — four
/// pixels of empty panel per row, growing with the panel because the
/// number of visible rows does (#708).
///
/// # This restates the formula, deliberately
///
/// Measuring a drawn row cannot catch it: the cursor advances by the
/// widget's size plus the spacing, so height and advance scale
/// together and the assertion holds either way. That is exactly what
/// the first attempt at this test did, and it passed with the bug
/// reinstated.
///
/// What is being pinned is not the formula but that **nothing is
/// added to it** — so the formula has to appear here for the addition
/// to be visible.
#[test]
fn the_row_height_excludes_the_spacing_show_rows_adds() {
    with_ui(|ui| {
        let line = ui.text_style_height(&egui::TextStyle::Button);
        let content =
            (line + 2.0 * ui.spacing().button_padding.y).max(ui.spacing().interact_size.y);
        assert!(ui.spacing().item_spacing.y > 0.0, "otherwise vacuous");
        assert!(
            (entity_row::row_height(ui) - content).abs() < 0.01,
            "row_height is {} but the content is {content}; anything extra is \
                 space egui will reserve and no row will fill",
            entity_row::row_height(ui),
        );
    });
}

/// The one invariant virtualization rests on. `show_rows` places every
/// row from an index times this pitch without drawing the rows above,
/// so a row that advances the cursor by anything else puts the whole
/// list out of step with the scrollbar — and clicks land on a
/// neighbour.
///
/// # Against the pitch, not the height
///
/// This compared the cursor's advance to `row_height` and passed,
/// which is how the bug in #708 survived being tested: the advance
/// includes `item_spacing.y`, so the test was asserting that the
/// height *is* the pitch — and `row_height` obliged by including the
/// spacing, leaving egui to add a second one.
#[test]
fn a_row_advances_the_cursor_by_exactly_one_pitch() {
    let entities = vec![entity_info(0, None)];
    let (reserved, occupied) = with_ui(|ui| {
        let reserved = row_pitch(ui, entity_row::row_height(ui));
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
            // A leaf: these rigs plant a single entity with no children.
            None,
        );
        (reserved, ui.cursor().top() - before)
    });
    assert!(
        (reserved - occupied).abs() < 0.01,
        "the list reserves {reserved} per row and the row advanced {occupied}",
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
        short_name: "Name".into(),
        fields: ReflectedFields::Values(vec![(
            "value".to_owned(),
            kooch_ecs::reflect::ReflectValue::String("x".repeat(400)),
        )]),
        field_metas: None,
        visibility: Default::default(),
    }];
    let entities = vec![long];

    let (reserved, occupied) = with_ui(|ui| {
        let reserved = row_pitch(ui, entity_row::row_height(ui));
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
            // A leaf: these rigs plant a single entity with no children.
            None,
        );
        (reserved, ui.cursor().top() - before)
    });
    assert!(
        (reserved - occupied).abs() < 0.01,
        "a 400-character name advanced the cursor {occupied}, not {reserved}",
    );
}

/// One entity with a child, for the collapse tests.
///
/// ⚠️ `base` is not decoration. The open flags are persisted per ENTITY,
/// and egui's store outlives one `build_rows`, so two rigs sharing entity
/// ids in the same `with_ui` share their expanded state — the second
/// reads whatever the first left behind. That is a real property of the
/// panel and it made this file's first draft pass for the wrong reason.
fn parent_and_child(scene: kooch_core::Guid, prefab: bool, base: u32) -> Vec<EntityDisplayInfo> {
    let mut parent = entity_info(base, Some(scene));
    let mut child = entity_info(base + 1, Some(scene));
    parent.is_prefab_instance = prefab;
    child.is_prefab_instance = prefab;
    parent.children = vec![child.entity];
    child.parent = Some(parent.entity);
    child.depth = 1;
    vec![parent, child]
}

fn entity_rows(rows: &[WorldRow]) -> usize {
    rows.iter()
        .filter(|row| matches!(row, WorldRow::Entity(_)))
        .count()
}

/// A collapsed parent's children are *absent* from the list, not skipped
/// while drawing.
///
/// 🔴 That is the whole point of building the rows from the open flags.
/// Skipping them one at a time leaves the cost proportional to the whole
/// world, which is exactly what collapsing exists to avoid — and with 36
/// prefab instances of five entities each, the difference is 36 rows
/// against 180.
#[test]
fn a_collapsed_parent_hides_its_subtree() {
    let id = kooch_core::Guid::new_v4();
    let entities = parent_and_child(id, false, 0);
    let scenes = vec![scene_info(id, true)];

    let (open, closed) = with_ui(|ui| {
        let open = entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default()));
        ui.data_mut(|data| data.insert_persisted(subtree_id(entities[0].entity), false));
        let closed = entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default()));
        (open, closed)
    });

    assert_eq!(
        open, 2,
        "the child was not listed while its parent was open"
    );
    assert_eq!(closed, 1, "the child survived its parent being collapsed");
}

/// A prefab instance starts collapsed; anything else starts open.
///
/// 🔴 An instance is a unit — its members are the prefab's business, not
/// the scene's. A hand-built hierarchy is the opposite: somebody put
/// those children there on purpose, and starting closed would hide their
/// own work from them.
#[test]
fn a_prefab_instance_starts_collapsed() {
    let id = kooch_core::Guid::new_v4();
    let scenes = vec![scene_info(id, true)];
    let plain = parent_and_child(id, false, 0);
    let instance = parent_and_child(id, true, 10);

    let (plain_rows, instance_rows) = with_ui(|ui| {
        (
            entity_rows(&build_rows(ui, &plain, &scenes, &WorldFilter::default())),
            entity_rows(&build_rows(ui, &instance, &scenes, &WorldFilter::default())),
        )
    });

    assert_eq!(plain_rows, 2, "a hand-built parent started closed");
    assert_eq!(instance_rows, 1, "a prefab instance started open");
}

/// The default is decided for the instance's ROOT, not for every entity
/// it owns.
///
/// `is_prefab_instance` is true for all five entities of an instance, so
/// reading it alone would start the pivot inside it collapsed as well —
/// and a user who expands the instance would find its insides still
/// folded, one click at a time, for no stated reason.
#[test]
fn only_the_instances_root_starts_collapsed() {
    let id = kooch_core::Guid::new_v4();
    let scenes = vec![scene_info(id, true)];
    let mut entities = parent_and_child(id, true, 20);
    // A grandchild under the instance's own child.
    let mut grandchild = entity_info(22, Some(id));
    grandchild.is_prefab_instance = true;
    grandchild.parent = Some(entities[1].entity);
    grandchild.depth = 2;
    entities[1].children = vec![grandchild.entity];
    entities.push(grandchild);

    let rows = with_ui(|ui| {
        // Open the root, and nothing else.
        ui.data_mut(|data| data.insert_persisted(subtree_id(entities[0].entity), true));
        entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default()))
    });

    assert_eq!(
        rows, 3,
        "expanding the instance's root did not reveal the whole instance"
    );
}

/// A selection inside a collapsed prefab instance gets a row to land on.
///
/// 🔴 Two things used to hide it, and both arrived with the tree. The
/// group reveal bailed on a single scene — right while a lone scene drew
/// no header, wrong the moment every scene got a root — and it only ever
/// opened the group, never the collapsed parents inside it. A prefab
/// instance starts collapsed, so duplicating one of its children put the
/// new entity in a subtree with no rows at all: selected, and nowhere on
/// screen (#706).
#[test]
fn a_reveal_opens_collapsed_ancestors() {
    let id = kooch_core::Guid::new_v4();
    let scenes = vec![scene_info(id, true)];
    let entities = parent_and_child(id, true, 30);
    let child = entities[1].entity;

    let rows = with_ui(|ui| {
        // Collapse the scene too, so both guards are under test.
        ui.data_mut(|data| data.insert_persisted(egui::Id::new(("world_group_open", id)), false));
        assert_eq!(
            entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default())),
            0,
            "nothing was hidden, so the test proves nothing",
        );
        super::reveal_group_of(ui, &entities, &scenes, child);
        entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default()))
    });

    assert_eq!(rows, 2, "the revealed child still had no row");
}

/// A scene's row offers Save; the "Unsaved" pseudo-group does not.
///
/// The group holding entities that belong to no scene is not a file, so
/// there is nowhere for it to be saved to — and offering it would write
/// its entities into whichever scene happened to be active, which is the
/// note under that header, not a menu item.
#[test]
fn only_a_scene_row_can_be_saved() {
    let id = kooch_core::Guid::new_v4();
    let scene = super::GroupHeader::scene(&scene_info(id, true), 3);
    assert_eq!(scene.scene, Some(id), "a scene row names its scene");
    assert_eq!(
        super::GroupHeader::unsaved(2).scene,
        None,
        "the unsaved group would have offered to save entities into a file it does not have",
    );
}

/// The unsaved marker leads the name.
///
/// The entity count sits between the name and the end of the line, so a
/// trailing marker is separated from what it describes by a number that
/// changes — and a column of scenes is read down its left edge.
#[test]
fn the_dirty_marker_leads_the_name() {
    let id = kooch_core::Guid::new_v4();
    let mut info = scene_info(id, true);
    assert!(
        !super::GroupHeader::scene(&info, 1).label.starts_with('*'),
        "a clean scene was marked",
    );
    info.dirty = true;
    let header = super::GroupHeader::scene(&info, 1);
    assert!(header.label.starts_with("*Scene"), "{}", header.label);
    assert!(
        header.dirty,
        "the row cannot say what its menu should offer"
    );
}

/// Every entity row sits one level deeper than the scene above it.
///
/// 🔴 Drawn at its own hierarchy depth, a root entity started in the same
/// column as its scene's header — so a scene with four roots read as five
/// scenes, and the one thing the tree exists to say went missing.
#[test]
fn an_entity_is_indented_under_its_scene() {
    use super::entity_row::indent_levels;
    assert_eq!(
        indent_levels(0),
        1,
        "a root entity sat level with its scene"
    );
    assert_eq!(
        indent_levels(2),
        3,
        "the offset was lost deeper in the tree"
    );
}

/// Dropping onto a collapsed entity opens the chain above it.
///
/// 🔴 Without this the dragged entity *vanishes*: the reparent works and
/// its row lands inside a subtree that is not listed. Nothing says where
/// it went, and the obvious reading is that the drag deleted it.
#[test]
fn a_drop_target_opens_up_to_its_root() {
    let id = kooch_core::Guid::new_v4();
    let scenes = vec![scene_info(id, true)];
    // root → middle → leaf, all collapsed.
    let mut root = entity_info(40, Some(id));
    let mut middle = entity_info(41, Some(id));
    let leaf = entity_info(42, Some(id));
    root.children = vec![middle.entity];
    middle.parent = Some(root.entity);
    middle.depth = 1;
    middle.children = vec![leaf.entity];
    let mut leaf = leaf;
    leaf.parent = Some(middle.entity);
    leaf.depth = 2;
    let entities = vec![root, middle, leaf];

    let rows = with_ui(|ui| {
        for e in &entities {
            ui.data_mut(|d| d.insert_persisted(subtree_id(e.entity), false));
        }
        assert_eq!(
            entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default())),
            1,
            "nothing was collapsed, so the test proves nothing",
        );
        // Dropping onto the leaf: its whole chain has to open.
        super::entity_row::reveal_chain(ui, entities[2].entity, &entities);
        entity_rows(&build_rows(ui, &entities, &scenes, &WorldFilter::default()))
    });

    assert_eq!(rows, 3, "the drop target's chain stayed folded");
}

/// A scene with no file offers no "Discard Changes".
///
/// There is nothing to revert *to*, and despawning its entities would
/// delete work rather than undo it — the one thing discard must never be
/// mistaken for.
#[test]
fn an_unsaved_scene_cannot_discard() {
    let id = kooch_core::Guid::new_v4();
    let mut info = scene_info(id, true);
    info.dirty = true;
    assert!(
        !super::GroupHeader::scene(&info, 1).has_file,
        "a scene that has never been saved claimed a file to revert to",
    );
    info.path = Some(std::path::PathBuf::from("scenes/station.scene"));
    assert!(super::GroupHeader::scene(&info, 1).has_file);
}

/// Which entities a filtered list ended up listing, by display index.
fn matched_indices(rows: &[WorldRow]) -> Vec<usize> {
    rows.iter()
        .filter_map(|row| match row {
            WorldRow::Entity(idx) => Some(*idx),
            _ => None,
        })
        .collect()
}

/// A component with no fields, for the type filter to find.
fn carrying(index: u32, scene: Option<kooch_core::Guid>, types: &[&str]) -> EntityDisplayInfo {
    let mut info = entity_info(index, scene);
    info.components = types
        .iter()
        .map(|name| crate::state::ComponentDisplayInfo {
            type_id: std::any::TypeId::of::<()>(),
            component: kooch_ecs::component::ComponentId::INVALID,
            short_name: (*name).to_owned().into(),
            fields: ReflectedFields::Values(Vec::new()),
            field_metas: None,
            visibility: Default::default(),
        })
        .collect();
    info
}

/// Gives an entity a `Name`, which is what the text filter reads.
fn called(mut info: EntityDisplayInfo, name: &str) -> EntityDisplayInfo {
    info.components.push(crate::state::ComponentDisplayInfo {
        type_id: std::any::TypeId::of::<()>(),
        component: kooch_ecs::component::ComponentId::INVALID,
        short_name: "Name".into(),
        fields: ReflectedFields::Values(vec![(
            "value".to_owned(),
            kooch_ecs::reflect::ReflectValue::String(name.to_owned()),
        )]),
        field_metas: None,
        visibility: Default::default(),
    });
    info
}

/// The question that cost a day and a half: how many directional lights
/// are in this scene? A name box cannot ask it.
#[test]
fn a_type_filter_finds_both_suns() {
    let scene = kooch_core::Guid::new_v4();
    let mut entities: Vec<_> = (0..500)
        .map(|i| carrying(i, Some(scene), &["MeshRenderer"]))
        .collect();
    entities.push(carrying(500, Some(scene), &["DirectionalLight"]));
    entities.push(carrying(501, Some(scene), &["DirectionalLight"]));
    let scenes = vec![scene_info(scene, true)];

    let filter = WorldFilter {
        text: String::new(),
        component: Some("DirectionalLight".to_owned()),
    };
    let rows = with_ui(|ui| matched_indices(&build_rows(ui, &entities, &scenes, &filter)));

    assert_eq!(rows, vec![500, 501]);
}

#[test]
fn a_name_filter_is_case_insensitive() {
    let scene = kooch_core::Guid::new_v4();
    let entities = vec![
        called(entity_info(0, Some(scene)), "Player"),
        called(entity_info(1, Some(scene)), "Ground"),
    ];
    let scenes = vec![scene_info(scene, true)];

    let filter = WorldFilter {
        text: "play".to_owned(),
        component: None,
    };
    let rows = with_ui(|ui| matched_indices(&build_rows(ui, &entities, &scenes, &filter)));

    assert_eq!(rows, vec![0]);
}

/// Two narrowings that widened each other would be a filter nobody
/// could predict.
#[test]
fn both_terms_narrow_together() {
    let scene = kooch_core::Guid::new_v4();
    let entities = vec![
        called(carrying(0, Some(scene), &["DirectionalLight"]), "Sun"),
        called(carrying(1, Some(scene), &["DirectionalLight"]), "Moon"),
        called(carrying(2, Some(scene), &["PointLight"]), "Sun lamp"),
    ];
    let scenes = vec![scene_info(scene, true)];

    let filter = WorldFilter {
        text: "sun".to_owned(),
        component: Some("DirectionalLight".to_owned()),
    };
    let rows = with_ui(|ui| matched_indices(&build_rows(ui, &entities, &scenes, &filter)));

    assert_eq!(rows, vec![0]);
}

/// A match hidden under a closed parent is a search that found the thing
/// and did not show it.
#[test]
fn a_filter_reaches_into_collapsed_subtrees() {
    let scene = kooch_core::Guid::new_v4();
    let mut parent = called(entity_info(0, Some(scene)), "Rig");
    let child = called(entity_info(1, Some(scene)), "Sun");
    parent.children = vec![child.entity];
    let entities = vec![parent, child];
    let scenes = vec![scene_info(scene, true)];

    let filter = WorldFilter {
        text: "sun".to_owned(),
        component: None,
    };
    let rows = with_ui(|ui| {
        // Closed, which without the filter hides row 1 entirely.
        ui.data_mut(|d| d.insert_persisted(subtree_id(entities[0].entity), false));
        matched_indices(&build_rows(ui, &entities, &scenes, &filter))
    });

    assert_eq!(rows, vec![1]);
}

/// An empty panel is indistinguishable from an empty world, so it says
/// which one it is.
#[test]
fn no_match_says_so() {
    let scene = kooch_core::Guid::new_v4();
    let entities = vec![called(entity_info(0, Some(scene)), "Player")];
    let scenes = vec![scene_info(scene, true)];

    let filter = WorldFilter {
        text: "nothing here".to_owned(),
        component: None,
    };
    let rows = with_ui(|ui| build_rows(ui, &entities, &scenes, &filter));

    assert!(
        matches!(rows.as_slice(), [WorldRow::Note(_)]),
        "{}",
        rows.len()
    );
}
