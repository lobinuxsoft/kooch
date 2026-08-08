use super::*;

/// A scene with nobody opting into filtering must behave as though
/// filtering did not exist.
#[test]
fn everything_interacts_by_default() {
    let mask = InteractionMask::default();
    assert!(mask.interacts_with(InteractionMask::default()));
    assert_eq!(mask, InteractionMask::ALL);
}

/// Acceptance: "two bodies in non-overlapping collision groups pass
/// through each other."
#[test]
fn disjoint_groups_do_not_interact() {
    let a = InteractionMask {
        memberships: 0b0001,
        filter: 0b0001,
    };
    let b = InteractionMask {
        memberships: 0b0010,
        filter: 0b0010,
    };
    assert!(!a.interacts_with(b));
    assert!(!b.interacts_with(a));
}

/// Both directions have to agree. This is the case that catches people
/// out: being in a group the other side filters for is not enough on
/// its own.
#[test]
fn interaction_needs_both_directions() {
    // `a` will interact with `b`'s group, but `b` filters for nothing
    // `a` is a member of.
    let a = InteractionMask {
        memberships: 0b0001,
        filter: 0b0010,
    };
    let b = InteractionMask {
        memberships: 0b0010,
        filter: 0b0100,
    };
    assert!(!a.interacts_with(b), "one-sided agreement is not enough");

    let b = InteractionMask {
        memberships: 0b0010,
        filter: 0b0001,
    };
    assert!(a.interacts_with(b), "mutual agreement should interact");
}

#[test]
fn nothing_interacts_with_none() {
    assert!(!InteractionMask::ALL.interacts_with(InteractionMask::NONE));
    assert!(!InteractionMask::NONE.interacts_with(InteractionMask::ALL));
}

/// Events are opt-in, which is why the engine heard nothing before
/// #561 — and the reason the cost is proportional to what a game
/// listens for.
#[test]
fn a_default_collider_asks_for_no_events() {
    assert!(!ColliderInteraction::default().wants_events());
    assert!(
        ColliderInteraction {
            collision_events: true,
            ..Default::default()
        }
        .wants_events()
    );
    assert!(
        ColliderInteraction {
            contact_force_events: true,
            ..Default::default()
        }
        .wants_events()
    );
}

#[test]
fn a_default_collider_is_solid_and_unfiltered() {
    let interaction = ColliderInteraction::default();
    assert!(!interaction.sensor);
    assert_eq!(interaction.collision_groups, InteractionMask::ALL);
    assert_eq!(interaction.solver_groups, InteractionMask::ALL);
}
