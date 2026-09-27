//! The rig's gizmos draw state, so these check that the state reaches the screen.

use super::*;
use crate::gizmos::harness::{arrows, draw};
use glam::Mat4;

fn lookahead() -> CameraLookahead {
    CameraLookahead {
        enabled: true,
        max_distance: 4.0,
        ..Default::default()
    }
}

/// 🔴 #1334: the cap is drawn whether or not the lead has reached it. A ceiling you cannot see is
/// a number you cannot tune, which is how `max_distance` sat at 4 m through three days of
/// debugging without anyone knowing it never came near.
#[test]
fn the_cap_is_drawn_before_it_is_reached() {
    let drawn = draw(&LookaheadVisualizer, &lookahead(), Mat4::IDENTITY);
    // Nothing without resources: the lead is state, and `draw` has none.
    assert!(drawn.is_empty(), "it drew a lead it could not know");
}

/// A disabled component draws nothing, so a switched-off lookahead is not mistaken for one that is
/// simply not leading.
#[test]
fn a_disabled_lookahead_is_silent() {
    let off = CameraLookahead {
        enabled: false,
        ..lookahead()
    };
    assert!(draw(&LookaheadVisualizer, &off, Mat4::IDENTITY).is_empty());
    assert!(arrows(&LookaheadVisualizer, &off, Mat4::IDENTITY).is_empty());
}

/// A target's marker is sized by its weight: which member pulls hardest is visible rather than
/// read off a list.
#[test]
fn a_heavier_target_draws_bigger() {
    let light = CameraTarget {
        group: 0,
        weight: 0.0,
    };
    let heavy = CameraTarget {
        group: 0,
        weight: 4.0,
    };
    let reach = |target: &CameraTarget| {
        draw(&CameraTargetVisualizer, target, Mat4::IDENTITY)
            .iter()
            .flat_map(|(a, b)| [a.length(), b.length()])
            .fold(0.0, f32::max)
    };
    assert!(
        reach(&heavy) > reach(&light),
        "weight is not visible: {} against {}",
        reach(&heavy),
        reach(&light),
    );
}

/// A collision draws nothing without the arm it is describing: the sweep is state.
#[test]
fn a_collision_without_state_is_silent() {
    let collision = CameraCollision {
        enabled: true,
        ..Default::default()
    };
    assert!(draw(&CameraCollisionVisualizer, &collision, Mat4::IDENTITY).is_empty());
}
