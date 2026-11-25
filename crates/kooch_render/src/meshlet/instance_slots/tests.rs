use kooch_ecs::entity::Entity;

use super::*;

fn drawn(ids: &[u32]) -> Vec<(MeshInstance, DrawKey)> {
    ids.iter()
        .map(|&id| {
            (
                MeshInstance::new(glam::Mat4::IDENTITY, id, 0),
                (Entity::new(id, 0), 0),
            )
        })
        .collect()
}

/// Every mesh has two LOD groups; mesh 99 is the empty one.
fn place(slots: &mut InstanceSlots, ids: &[u32]) {
    slots.place(drawn(ids), |_| 2, 99);
}

#[test]
fn a_leaving_row_moves_nothing() {
    let mut slots = InstanceSlots::default();
    place(&mut slots, &[1, 2, 3]);
    let before = slots.rows()[2];
    place(&mut slots, &[1, 3]);
    let after = slots.rows()[2];
    assert_eq!(
        (after.mesh_id, after.group_base),
        (before.mesh_id, before.group_base)
    );
    assert_eq!(slots.rows()[1].layers, 0, "the freed row culls to nothing");
    assert_eq!(slots.rows()[1].mesh_id, 99);
}

#[test]
fn a_freed_slot_is_reused() {
    let mut slots = InstanceSlots::default();
    place(&mut slots, &[1, 2, 3]);
    place(&mut slots, &[1, 3, 4]);
    assert_eq!(slots.rows().len(), 3);
    assert_eq!(slots.rows()[1].mesh_id, 4);
    assert_eq!(slots.groups(), 6);
}

/// A row that stops moving is written once more, for its previous transform, then never again.
#[test]
fn a_still_row_stops_uploading() {
    let mut slots = InstanceSlots::default();
    let mut written = Vec::new();
    for x in [0.0, 1.0, 1.0, 1.0] {
        let moved = glam::Mat4::from_translation(glam::Vec3::X * x);
        let row = MeshInstance::new(moved, 1, 0);
        slots.place([(row, (Entity::new(1, 0), 0))], |_| 2, 99);
        slots.diff();
        written.push(slots.runs().to_vec());
    }
    assert_eq!(written, [vec![0..1], vec![0..1], vec![0..1], vec![]]);
}

#[test]
fn a_kept_row_outlives_place() {
    let mut slots = InstanceSlots::default();
    let kept = MeshInstance::new(glam::Mat4::IDENTITY, 7, 0);
    slots.keep(kept, (Entity::new(7, 0), 1), 2);
    place(&mut slots, &[1]);
    place(&mut slots, &[1]);
    assert!(slots.rows().iter().any(|row| row.mesh_id == 7));
    slots.unkeep((Entity::new(7, 0), 1));
    place(&mut slots, &[1]);
    assert!(slots.rows().iter().all(|row| row.mesh_id != 7));
}
