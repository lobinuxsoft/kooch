use glam::Vec3;
use kooch_blockmesh::{BlockShape, Shape};

use super::{Field, closest_along, dragged, handles};

/// Every handle of every default shape, as a (shape, handle) list.
fn every_handle() -> Vec<(BlockShape, super::Handle)> {
    Shape::DEFAULTS
        .iter()
        .flat_map(|shape| {
            let block = BlockShape::from(*shape);
            handles(&block)
                .into_iter()
                .filter(|handle| handle.around.is_none())
                .map(move |handle| (block, handle))
        })
        .collect()
}

/// 🔴 A drag puts the handle where the cursor took it: after writing the dragged value, the handle
/// has moved exactly the dragged distance along its axis. Checked for every handle of every kind.
#[test]
fn a_handle_follows_the_drag() {
    for (block, handle) in every_handle() {
        let value = dragged(&block, handle.field, 0.3, None).expect("the handle moves");
        let mut moved = block;
        handle.field.set(&mut moved, value);
        let after = handles(&moved)
            .into_iter()
            .find(|h| h.field == handle.field)
            .expect("still there");
        let travelled = (after.at - handle.at).dot(handle.axis);
        assert!(
            (travelled - 0.3).abs() < 1.0e-3,
            "{:?} on kind {} travelled {travelled}",
            handle.field,
            block.kind
        );
    }
}

/// The pivot moves the mesh, so it changes how far a handle travels per unit: a cube's +X face moves
/// half its size growth around a centred pivot and all of it from a corner.
#[test]
fn the_pivot_changes_the_handle_rate() {
    let centred = BlockShape {
        pivot: Vec3::ZERO,
        ..BlockShape::default()
    };
    let corner = BlockShape {
        pivot: Vec3::NEG_ONE,
        ..BlockShape::default()
    };
    let grow = |block: &BlockShape| dragged(block, Field::SizeX, 0.5, None).unwrap() - block.size.x;
    assert!((grow(&centred) - 1.0).abs() < 1.0e-3);
    assert!((grow(&corner) - 0.5).abs() < 1.0e-3);
}

#[test]
fn a_snapped_drag_lands_on_the_step() {
    let value = dragged(&BlockShape::default(), Field::SizeY, 0.37, Some(0.25)).unwrap();
    assert!((value / 0.25 - (value / 0.25).round()).abs() < 1.0e-4);
}

#[test]
fn a_drag_through_zero_stops_at_the_floor() {
    let value = dragged(&BlockShape::default(), Field::SizeY, -50.0, None).unwrap();
    assert!(value > 0.0);
}

/// A ray passing straight across the Y axis at height 2 is closest to the axis at 2.
#[test]
fn the_closest_point_on_an_axis() {
    let along = closest_along(Vec3::ZERO, Vec3::Y, Vec3::new(-5.0, 2.0, 1.0), Vec3::X).unwrap();
    assert!((along - 2.0).abs() < 1.0e-5);
}

/// 🔴 From the smoke test: a pivot on the far side pinned the face a handle sat on, and the arch's
/// and door's depth handles did nothing. Every linear handle, under pivots on every side.
#[test]
fn every_handle_moves_under_any_pivot() {
    let pivots = [
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::ZERO,
    ];
    let turning = BlockShape {
        turn: 120.0,
        ..BlockShape::from(Shape::DEFAULTS[1])
    };
    let shapes = Shape::DEFAULTS
        .iter()
        .map(|shape| BlockShape::from(*shape))
        .chain(std::iter::once(turning));
    for base in shapes {
        for pivot in pivots {
            let block = BlockShape { pivot, ..base };
            for handle in handles(&block).into_iter().filter(|h| h.around.is_none()) {
                let value = dragged(&block, handle.field, 0.1, None).unwrap_or_else(|| {
                    panic!(
                        "{:?} on kind {} at {pivot} is stuck",
                        handle.field, block.kind
                    )
                });
                let mut moved = block;
                handle.field.set(&mut moved, value);
                let after = handles(&moved)
                    .into_iter()
                    .find(|h| h.field == handle.field)
                    .expect("still there");
                let travelled = (after.at - handle.at).dot(handle.axis);
                assert!(
                    (travelled - 0.1).abs() < 1.0e-3,
                    "{:?} on kind {} at {pivot} travelled {travelled}",
                    handle.field,
                    block.kind
                );
            }
        }
    }
}

/// Stairs offer a turn handle straight or turning, so a flight can be bent from zero.
#[test]
fn stairs_always_offer_a_turn() {
    for turn in [0.0, 90.0, 540.0] {
        let block = BlockShape {
            turn,
            ..BlockShape::from(Shape::DEFAULTS[1])
        };
        assert!(
            handles(&block)
                .iter()
                .any(|h| h.field == Field::Turn && h.around.is_some())
        );
    }
}
