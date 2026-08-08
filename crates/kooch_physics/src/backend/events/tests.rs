use super::*;
use slotmap::SlotMap;

fn two() -> (BodyHandle, BodyHandle) {
    let mut bodies: SlotMap<BodyHandle, ()> = SlotMap::with_key();
    (bodies.insert(()), bodies.insert(()))
}

/// A sensor's event carries no contact behind it, and a listener has to
/// be able to tell before it goes looking.
#[test]
fn a_collision_event_says_whether_it_was_a_sensor() {
    let (a, b) = two();
    let solid = CollisionEvent {
        a,
        b,
        started: true,
        sensor: false,
    };
    let overlap = CollisionEvent {
        sensor: true,
        ..solid
    };
    assert!(!solid.sensor);
    assert!(overlap.sensor);
    assert_ne!(solid, overlap);
}

/// Total and peak are different questions — a blow spread over many
/// points and a spike through one can share a total.
#[test]
fn a_force_event_reports_total_and_peak_separately() {
    let (a, b) = two();
    let event = ContactForceEvent {
        a,
        b,
        total_force_magnitude: 90.0,
        max_force_magnitude: 80.0,
    };
    assert!(event.max_force_magnitude <= event.total_force_magnitude);
}
