use super::*;

/// The corner is a checker of two greys, so pixelation and blur have an edge to show on.
#[test]
fn the_corner_is_a_checker() {
    assert_eq!(test_pixel(250, 250, TEST_SIDE), [30, 30, 30, 255]);
    assert_eq!(test_pixel(242, 250, TEST_SIDE), [225, 225, 225, 255]);
}

/// The top row runs the wheel at full brightness, and the bottom row is dark.
#[test]
fn the_gradient_spans_hue_and_brightness() {
    assert_eq!(test_pixel(0, 0, TEST_SIDE), [255, 0, 0, 255]);
    let bottom = test_pixel(0, TEST_SIDE - 1, TEST_SIDE);
    assert!(bottom[0] < 2, "{bottom:?}");
}
