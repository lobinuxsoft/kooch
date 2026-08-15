/// The ordering the frame loop exists to get right, pinned against the
/// source itself.
///
/// There is no headless surface, so nothing in the test suite can watch
/// a real `get_current_texture` block. What can be checked is the thing
/// that would silently regress: the scene must be recorded and submitted
/// **before** the swapchain image is asked for.
///
/// 🔴 Reversing these two is invisible. Every pixel is identical, every
/// other test still passes, and the only symptom is a frame that adds
/// its recording time to its GPU time instead of hiding one behind the
/// other — 3.006 ms on top of 34 on the OneXFly, and no overlap between
/// this frame's CPU work and the last frame's GPU work at all.
///
/// The same idiom `kooch_lighting`'s own tests use to pin a line of
/// shader source: crude, and it fails the day someone moves the acquire
/// back up, which is the entire point.
#[test]
fn the_scene_is_submitted_before_the_image_is_asked_for() {
    let source = include_str!("mod.rs");

    let scene = source
        .find("render_with_assets_primary(")
        .expect("the frame loop still renders the scene");
    let acquire = source
        .find("get_current_texture()")
        .expect("the frame loop still acquires a swapchain image");

    assert!(
        scene < acquire,
        "the swapchain image is acquired before the scene is submitted: the \
         meshlet stage draws into its own textures and needs no surface, so \
         acquiring first makes the CPU wait out the compositor before \
         recording work the compositor has nothing to do with",
    );
}

/// The counterpart, so the test above cannot pass by accident on a file
/// that stopped acquiring or stopped rendering.
#[test]
fn the_frame_loop_still_does_both() {
    let source = include_str!("mod.rs");
    assert_eq!(
        source.matches("get_current_texture()").count(),
        1,
        "one acquire per frame, and one place to keep after the scene",
    );
    assert_eq!(
        source.matches("render_with_assets_primary(").count(),
        1,
        "one scene render per frame",
    );
}
