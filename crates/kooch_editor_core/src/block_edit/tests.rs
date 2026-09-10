use super::{BlockSelection, ElementMode};
use kooch_ecs::entity::Entity;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn object_is_the_default_mode() {
    // Every other kind of entity wants clicks to select entities.
    assert_eq!(ElementMode::default(), ElementMode::Object);
}

#[test]
fn a_fresh_selection_holds_nothing() {
    let selection = BlockSelection::default();
    assert!(selection.is_empty());
    assert!(!selection.holds(entity(1), 0));
}

#[test]
fn only_replaces_what_was_held() {
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    selection.only(entity(1), 5);
    assert_eq!(selection.elements, vec![5]);
}

#[test]
fn toggle_adds_then_removes() {
    let mut selection = BlockSelection::default();
    selection.toggle(entity(1), 2);
    selection.toggle(entity(1), 4);
    assert_eq!(selection.elements, vec![2, 4]);
    selection.toggle(entity(1), 2);
    assert_eq!(selection.elements, vec![4]);
}

#[test]
fn another_block_clears_rather_than_merges() {
    // 🔴 The faces are indices into ONE mesh. Keeping the previous
    // block's would address faces that may not exist in this one.
    let mut selection = BlockSelection::default();
    selection.toggle(entity(1), 5);
    selection.toggle(entity(2), 0);
    assert_eq!(selection.entity, Some(entity(2)));
    assert_eq!(selection.elements, vec![0]);
}

#[test]
fn a_face_of_another_block_is_not_held() {
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    assert!(selection.holds(entity(1), 3));
    assert!(!selection.holds(entity(2), 3));
}

#[test]
fn clearing_empties_both_halves() {
    let mut selection = BlockSelection::default();
    selection.only(entity(1), 3);
    selection.clear();
    assert!(selection.is_empty());
    assert_eq!(selection.entity, None);
}

#[test]
fn a_click_on_a_face_selects_it() {
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    assert_eq!(selection.elements, vec![2]);
}

#[test]
fn a_ctrl_click_adds_to_the_selection() {
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    super::apply_click(&mut selection, entity(1), Some(4), true);
    assert_eq!(selection.elements, vec![2, 4]);
}

#[test]
fn a_click_on_nothing_clears() {
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    super::apply_click(&mut selection, entity(1), None, false);
    assert!(selection.is_empty());
}

#[test]
fn a_ctrl_click_on_nothing_keeps_it() {
    // A miss, not "deselect everything" — the same rule entity picking
    // follows, so building a selection does not depend on aim.
    let mut selection = BlockSelection::default();
    super::apply_click(&mut selection, entity(1), Some(2), false);
    super::apply_click(&mut selection, entity(1), None, true);
    assert_eq!(selection.elements, vec![2]);
}

/// 🔴 Object mode stops the geometry being edited.
///
/// The gate was on whether a selection existed, not on the mode — so
/// switching to Object left the handle reshaping the block while the
/// author was asking to move it. Leaving faces selected while switching
/// is how you check what you just built.
#[test]
fn object_mode_is_not_face_mode() {
    assert_ne!(ElementMode::Object, ElementMode::Face);
    assert_eq!(ElementMode::Object.label(), "Object");
    assert_eq!(ElementMode::Face.label(), "Face");
}

/// Resources holding one element selected in `mode`.
///
/// The mode is part of the fixture, not a default: nothing can be
/// selected before the mode that reads the index is on, so a selection
/// carrying `Object` is a state the editor cannot reach.
fn with_selection(mode: ElementMode) -> kooch_core::resource::Resources {
    let mut resources = kooch_core::resource::Resources::new();
    let mut selection = BlockSelection {
        mode,
        ..Default::default()
    };
    selection.only(entity(1), 3);
    resources.insert(selection);
    resources
}

fn held(resources: &kooch_core::resource::Resources) -> bool {
    !resources
        .get::<BlockSelection>()
        .map(|s| s.is_empty())
        .unwrap_or(true)
}

#[test]
fn face_mode_keeps_the_selection() {
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, false);
    assert!(held(&resources));
}

#[test]
fn object_mode_drops_the_selection() {
    // 🔴 Cleared, not merely gated. A painted highlight over a grabbable
    // gizmo that records nothing looks like it worked.
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    assert!(!held(&resources));
}

#[test]
fn a_mode_switch_drops_the_selection() {
    // 🔴 Face 3 and edge 3 are both `3`. Carrying the indices across a
    // switch between element modes leaves unrelated geometry lit and
    // draggable, and nothing about it looks wrong.
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Edge, false);
    assert!(!held(&resources));
}

#[test]
fn play_drops_the_selection() {
    // The world Play restores is not the one these face indices were
    // read from.
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, true);
    assert!(!held(&resources));
}

#[test]
fn dropping_twice_is_quiet() {
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    assert!(!held(&resources));
}

#[test]
fn face_mode_reaches_the_drawing() {
    // 🔴 The gizmo reads Resources, and the overlay is taken OUT of
    // Resources for the frame that draws it. The mode has to be
    // mirrored somewhere the drawing can see, or the wireframe never
    // appears however the toolbar looks.
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, false);
    assert_eq!(
        resources.get::<BlockSelection>().map(|s| s.mode),
        Some(ElementMode::Face),
    );
}

#[test]
fn object_mode_reaches_it_too() {
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Object, false);
    assert_eq!(
        resources.get::<BlockSelection>().map(|s| s.mode),
        Some(ElementMode::Object),
    );
}

#[test]
fn play_reads_as_object_to_the_drawing() {
    // Nothing is editable during Play, so nothing should look editable.
    let mut resources = with_selection(ElementMode::Face);
    super::drop_selection_unless_editing(&mut resources, ElementMode::Face, true);
    assert_eq!(
        resources.get::<BlockSelection>().map(|s| s.mode),
        Some(ElementMode::Object),
    );
}

/// A unit cube: eight corners, six faces, twelve edges.
fn cube() -> kooch_blockmesh::BlockMesh {
    kooch_blockmesh::BlockMesh::cuboid(glam::Vec3::splat(0.5))
}

#[test]
fn a_face_yields_four_corners() {
    let mesh = cube();
    assert_eq!(super::corners_of(&mesh, ElementMode::Face, &[0]).len(), 4);
}

#[test]
fn an_edge_yields_two_corners() {
    let mesh = cube();
    let corners = super::corners_of(&mesh, ElementMode::Edge, &[0]);
    assert_eq!(corners.len(), 2);
    let adjacency = kooch_blockmesh::Adjacency::of(&mesh);
    let ends = adjacency.edge_corners(0).expect("a real edge");
    assert!(ends.iter().all(|end| corners.contains(end)));
}

#[test]
fn a_vertex_yields_itself() {
    let mesh = cube();
    assert_eq!(super::corners_of(&mesh, ElementMode::Vertex, &[5]), vec![5]);
}

#[test]
fn a_missing_corner_is_dropped() {
    // An index past the mesh would panic the transform that follows.
    let mesh = cube();
    assert!(super::corners_of(&mesh, ElementMode::Vertex, &[999]).is_empty());
}

#[test]
fn a_shared_corner_is_listed_once() {
    // 🔴 The whole point of the authoring mesh. A cube's corner belongs
    // to three faces, and a list that named it three times would move it
    // three times and tear the block along the seams the shared
    // positions exist to prevent.
    let mesh = cube();
    let every_face: Vec<u32> = (0..mesh.face_count() as u32).collect();
    let corners = super::corners_of(&mesh, ElementMode::Face, &every_face);
    assert_eq!(corners.len(), mesh.positions().len());

    let mut seen = corners.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), corners.len());
}

#[test]
fn two_edges_share_their_corner_once() {
    let mesh = cube();
    let adjacency = kooch_blockmesh::Adjacency::of(&mesh);
    // Two edges that meet: they must contribute three corners, not four.
    let [a, b] = adjacency.edge_corners(0).expect("a real edge");
    let neighbour = (0..adjacency.edge_count() as u32)
        .find(|edge| {
            *edge != 0
                && adjacency
                    .edge_corners(*edge)
                    .is_some_and(|ends| ends.contains(&b) && !ends.contains(&a))
        })
        .expect("an edge continues from the other end");
    assert_eq!(
        super::corners_of(&mesh, ElementMode::Edge, &[0, neighbour]).len(),
        3
    );
}

#[test]
fn moving_a_face_leaves_the_far_one() {
    // The test the issue asks for: drag one face of a cube and the
    // opposite face must not follow, while the four beside it reshape.
    let mut mesh = cube();
    let top = (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().y > 0.99)
        .expect("a cube has a top") as u32;
    let bottom = (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().y < -0.99)
        .expect("a cube has a bottom") as u32;

    let low_before = mesh.centre_of(&[bottom]).unwrap();
    let corners = super::corners_of(&mesh, ElementMode::Face, &[top]);
    mesh.move_corners(&corners, glam::Vec3::Y);

    assert_eq!(mesh.centre_of(&[bottom]).unwrap(), low_before);
    assert!((mesh.centre_of(&[top]).unwrap().y - 1.5).abs() < 1e-5);
}

#[test]
fn moving_a_vertex_moves_one_corner() {
    let mut mesh = cube();
    let before = mesh.positions().to_vec();
    let corners = super::corners_of(&mesh, ElementMode::Vertex, &[2]);
    mesh.move_corners(&corners, glam::Vec3::X);

    let moved = (0..before.len())
        .filter(|index| mesh.positions()[*index] != before[*index])
        .count();
    assert_eq!(moved, 1);
}
