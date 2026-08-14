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
        let rows = build_rows(ui, &entities, &[]);
        // The list has been sitting at the top.
        ui.data_mut(|d| d.insert_temp(visible_range_id(), (0usize, 20usize)));
        let focus = newly_focused(ui, &selected).expect("selection is new");
        scroll_offset_for(ui, &rows, &entities, focus, 20.0)
            .map(|offset| (offset, row_pitch(ui, 20.0)))
    });

    let (offset, pitch) = offset.expect("a row 900 places down is not on screen");
    // Centred on row 900 with twenty rows visible, measured in the
    // pitch `show_rows` uses — height *plus* item spacing.
    assert!(
        (offset - (900.0 - 9.5) * pitch).abs() < 1.0,
        "expected the row centred, got offset {offset} at pitch {pitch}",
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
        let rows = build_rows(ui, &entities, &[]);
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
        let closed = build_rows(ui, &entities, &scenes);
        assert!(
            !closed.iter().any(|row| matches!(row, WorldRow::Entity(idx)
                    if entities[*idx].entity == hidden)),
            "a closed group contributes no entity rows",
        );

        reveal_group_of(ui, &entities, &scenes, hidden);
        let opened = build_rows(ui, &entities, &scenes);
        let index = opened.iter().position(|row| {
            matches!(row, WorldRow::Entity(idx)
                if entities[*idx].entity == hidden)
        });
        assert!(index.is_some(), "revealing the group gave it a row");
    });
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
        );
        (reserved, ui.cursor().top() - before)
    });
    assert!(
        (reserved - occupied).abs() < 0.01,
        "a 400-character name advanced the cursor {occupied}, not {reserved}",
    );
}
