//! 🔴 The one thing here a smoke test cannot see: whether a mark is at the arithmetically right
//! place. A mark drawn confidently in the wrong spot looks exactly like a mark drawn right — which
//! is how `Lens::new(60.0, 16.0 / 9.0)` stayed hardcoded in this overlay until #1254, in the
//! overlay whose job is finding framing bugs. Everything else about this feature is visible.
//!
//! ⚠️ **What these do NOT cover**: which camera `Screen` is built FROM. They build it by hand, so
//! the first version of this overlay — projecting from the selected vcam rather than from the
//! camera that drew the picture — was green here and visibly wrong on screen the moment a vcam
//! that was not winning the election got selected. The variable of interest held still, which is
//! this repo's recurring test defect, and a smoke test found it in minutes. What guards that now
//! is `CameraStack::read` being the same call `viewport/game.rs` renders through.

use super::*;

/// A camera at the origin looking down -Z, with a lens that is neither 60° nor 16:9.
fn screen() -> Screen {
    Screen {
        at: Vec3::ZERO,
        right: Vec3::X,
        above: Vec3::Y,
        forward: Vec3::NEG_Z,
        // 90° vertical and 1:1. 🔴 `Lens::span` is the screen's FULL size, not half of it, so at
        // 10 m depth the picture is 20 m across and the edge sits 10 m from the centre. Getting
        // that backwards is what this test caught while it was being written.
        lens: Lens::new(90.0, 1.0),
    }
}

#[test]
fn an_edge_point_lands_on_the_edge() {
    // Ten metres up and right at 10 m depth is the corner: ±0.5 by this field's documented
    // meaning.
    let on = screen().of(Vec3::new(10.0, 10.0, -10.0)).expect("in front");
    assert!(
        (on.x - 0.5).abs() < 1e-4 && (on.y - 0.5).abs() < 1e-4,
        "a corner point landed at {on}",
    );
}

#[test]
fn a_point_behind_is_not_on_screen() {
    assert!(screen().of(Vec3::new(0.0, 0.0, 10.0)).is_none());
}

/// The cap ring: a world distance becoming a fraction of the picture.
#[test]
fn a_metre_cap_scales_with_depth() {
    let screen = screen();
    let near = screen
        .across(Vec3::new(0.0, 0.0, -10.0), 5.0)
        .expect("in front");
    let far = screen
        .across(Vec3::new(0.0, 0.0, -20.0), 5.0)
        .expect("in front");
    // 5 m across a 20 m picture is a quarter of it.
    assert!((near - 0.25).abs() < 1e-4, "5 m at 10 m read as {near}");
    // Twice as far is half as wide on screen, which is the whole point of doing it per depth.
    assert!((far - 0.125).abs() < 1e-4, "5 m at 20 m read as {far}");
}
