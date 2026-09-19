//! Filtering the list by type and name, ranges inside a filter, and the Unsaved group's menu.

use super::*;

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

/// Two narrowings that widened each other would be a filter nobody could predict.
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

/// A match hidden under a closed parent is a search that found the thing and did not show it.
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

/// An empty panel is indistinguishable from an empty world, so it says which one it is.
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

/// 🔴 A range spans what is on SCREEN, not what is in the display list.
#[test]
fn a_shift_range_stays_inside_the_filter() {
    use super::entity_row::listed_range;

    // Rows 3, 7 and 900 survived the filter; everything else is hidden.
    let listed = [3usize, 7, 900];

    let span = listed_range(&listed, 3, 900).expect("both ends are listed");

    assert_eq!(span, &[3, 7, 900]);
}

/// An anchor the filter has since removed is not an end of a range.
#[test]
fn a_range_from_a_hidden_anchor_is_no_range() {
    use super::entity_row::listed_range;

    let listed = [3usize, 7, 900];

    assert!(listed_range(&listed, 42, 900).is_none());
}

/// The "Unsaved" group used to suppress its whole menu because it is not
/// a file. It stands over every scene-less entity, so with no scene open
/// that left the panel with nowhere to put anything (#1033).
#[test]
fn the_unsaved_group_offers_a_menu() {
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(400.0, 600.0),
    ));
    // The pseudo-group's own row, which is the first line the panel draws when no scene is open.
    let at = egui::pos2(200.0, 84.0);

    let draw = |input: egui::RawInput| {
        let mut entities = vec![entity_info(0, None)];
        let mut selected = Vec::new();
        let mut pinned = std::collections::HashSet::new();
        let mut actions = Vec::new();
        let mut last_clicked = None;
        ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_world_content(
                    ui,
                    true,
                    &mut entities,
                    &mut selected,
                    &mut pinned,
                    &[],
                    &mut actions,
                    1,
                    1,
                    1,
                    &mut last_clicked,
                    &[],
                    false,
                );
            });
        });
    };
    let plain = || egui::RawInput {
        screen_rect: screen,
        ..Default::default()
    };
    let secondary = |pressed| egui::RawInput {
        screen_rect: screen,
        events: vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        }],
        ..Default::default()
    };

    // Two settling frames: the row list has to exist before a click can
    // land on it, and the scroll area sizes itself on the second.
    draw(plain());
    draw(plain());
    draw(egui::RawInput {
        screen_rect: screen,
        events: vec![egui::Event::PointerMoved(at)],
        ..Default::default()
    });
    draw(secondary(true));
    draw(secondary(false));
    draw(plain());

    let open = ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .iter()
            .any(|layer| format!("{:?}", layer.id).contains("popup"))
    });
    assert!(
        open,
        "right-clicking the Unsaved row must offer somewhere to put an entity"
    );
}

/// An entity in no scene still has a root to put a sibling at.
#[test]
fn a_scene_less_row_spawns_at_root() {
    use crate::actions::SpawnTarget;

    let scene = kooch_core::Guid::new_v4();
    assert_eq!(
        entity_row::root_target(Some(scene)).1,
        SpawnTarget::Scene(scene)
    );
    assert_eq!(entity_row::root_target(None).1, SpawnTarget::Active);
}
