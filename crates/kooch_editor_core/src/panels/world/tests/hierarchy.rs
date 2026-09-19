//! Parents, instances and scenes in the tree: collapsing, revealing, saving, dirt, indentation.

use super::*;

/// One entity with a child, for the collapse tests.
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

/// A collapsed parent's children are *absent* from the list, not skipped while drawing.
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

/// The default is decided for the instance's ROOT, not for every entity it owns.
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
