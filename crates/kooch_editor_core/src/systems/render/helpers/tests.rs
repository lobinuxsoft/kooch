use super::{SAME_SPOT, in_turn};
use crate::state::ClickCycle;
use glam::Vec2;
use kooch_ecs::entity::Entity;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

fn after(picked: Entity) -> Option<ClickCycle> {
    Some(ClickCycle {
        at: Vec2::ZERO,
        picked,
    })
}

/// 🔴 It wraps. Without this the cycle stops on the last entity and a second click on the
/// front one never comes back — which reads as the click having broken.
#[test]
fn a_second_click_goes_deeper() {
    let under = [entity(1), entity(2), entity(3)];
    assert_eq!(in_turn(None, Vec2::ZERO, &under), Some(entity(1)));
    assert_eq!(
        in_turn(after(entity(1)), Vec2::ZERO, &under),
        Some(entity(2))
    );
    assert_eq!(
        in_turn(after(entity(3)), Vec2::ZERO, &under),
        Some(entity(1))
    );
}

/// Clicking somewhere else is a new click, not the next step of the old one.
#[test]
fn a_moved_cursor_starts_over() {
    let under = [entity(1), entity(2)];
    let far = Vec2::new(SAME_SPOT + 1.0, 0.0);
    assert_eq!(in_turn(after(entity(1)), far, &under), Some(entity(1)));
    // A hand holding a mouse drifts; a cycle that resets on one pixel never reaches the second.
    let drift = Vec2::new(SAME_SPOT - 1.0, 0.0);
    assert_eq!(in_turn(after(entity(1)), drift, &under), Some(entity(2)));
}

/// The remembered entity is gone — deleted, or moved out of the ray. Starting over beats
/// picking whatever took its place in the list.
#[test]
fn a_stale_pick_starts_over() {
    let under = [entity(7), entity(8)];
    assert_eq!(
        in_turn(after(entity(1)), Vec2::ZERO, &under),
        Some(entity(7))
    );
    assert_eq!(in_turn(after(entity(1)), Vec2::ZERO, &[]), None);
}
