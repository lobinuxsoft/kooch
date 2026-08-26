//! The grid's arithmetic: dimensions from a viewport, and the
//! logarithmic slice a depth falls in.
//!
//! 🔴 In their own file rather than an inline `mod tests`: the
//! vendoriser strips test code by matching a literal `#[cfg(test)]` on
//! its own line, and an inline module ships the tests into every
//! project built against the engine. `the_vendored_engine_contains_no_test_code`
//! is what catches it.

use super::*;

fn grid() -> ClusterGrid {
    ClusterGrid::new(&ClusterSettings::default(), Vec2::new(1280.0, 720.0))
}

#[test]
fn a_wide_viewport_gets_more_columns() {
    let g = grid();
    assert!(g.dimensions.x > g.dimensions.y);
    assert_eq!(g.dimensions.z, 24);
    // The budget is a budget: rounding to whole cells never grows it.
    assert!(g.cluster_count() <= ClusterSettings::default().total);
}

#[test]
fn a_thin_viewport_still_has_cells() {
    let g = ClusterGrid::new(&ClusterSettings::default(), Vec2::new(2000.0, 1.0));
    assert!(g.cluster_count() > 0);
    assert!(g.dimensions.y >= 1);
}

#[test]
fn the_first_slice_holds_the_near_field() {
    let g = grid();
    // Anything closer than `first_slice` is slice 0, and so is
    // anything the camera is sitting inside.
    assert_eq!(g.z_slice(-0.001), 0);
    assert_eq!(g.z_slice(-1.0), 0);
    assert_eq!(g.z_slice(-4.99), 0);
    // And the metre after it is not.
    assert_eq!(g.z_slice(-5.01), 1);
}

#[test]
fn slices_grow_with_distance() {
    let g = grid();
    // The span of one slice at 10 m against the span at 100 m: a
    // logarithmic distribution makes the far one much thicker. A
    // linear split would make them equal, which is the bug this
    // asserts against.
    let near_span = span_of(&g, g.z_slice(-10.0));
    let far_span = span_of(&g, g.z_slice(-100.0));
    assert!(far_span > near_span * 4.0, "{far_span} vs {near_span}");
}

#[test]
fn beyond_the_far_plane_lands_in_the_last_slice() {
    let g = grid();
    let last = g.dimensions.z - 1;
    assert_eq!(g.z_slice(-g.far), last);
    // The documented cost of having no readback: everything past the
    // grid piles into the final cell rather than being dropped.
    assert_eq!(g.z_slice(-100_000.0), last);
}

/// The depth range a slice covers, by walking outwards until the
/// mapping reports a different one.
fn span_of(g: &ClusterGrid, slice: u32) -> f32 {
    let mut z = -g.near;
    let mut start = None;
    while z > -g.far {
        if g.z_slice(z) == slice {
            if start.is_none() {
                start = Some(z);
            }
        } else if let Some(s) = start {
            return s - z;
        }
        z *= 1.001;
    }
    0.0
}

/// The slice a depth lands in and the thickness reported for that depth
/// have to describe the same slice, or the panel states a number the
/// grid does not use.
#[test]
fn depth_matches_the_slice_it_reports() {
    let g = grid();
    for distance in [6.0_f32, 15.0, 40.0, 120.0] {
        if !g.slice_depth(distance).is_finite() {
            continue;
        }
        let depth = g.slice_depth(distance);
        assert!(depth > 0.0, "slice at {distance} m has no thickness");
        // Stepping back by the reported thickness must not skip a slice.
        let here = g.z_slice(-distance);
        let further = g.z_slice(-(distance + depth * 0.9));
        assert!(
            further <= here + 1,
            "at {distance} m the thickness {depth} spans more than one slice",
        );
    }
}

/// 🔴 The point of #820: slices are thinner when the grid does not reach
/// as far, because the same 24 of them cover less distance.
#[test]
fn a_nearer_far_thins_the_slices() {
    let wide = ClusterGrid::new(
        &ClusterSettings {
            far: 200.0,
            ..Default::default()
        },
        Vec2::new(1280.0, 720.0),
    );
    let tight = ClusterGrid::new(
        &ClusterSettings {
            far: 40.0,
            ..Default::default()
        },
        Vec2::new(1280.0, 720.0),
    );
    let (wide_depth, tight_depth) = (wide.slice_depth(15.0), tight.slice_depth(15.0));
    assert!(
        tight_depth < wide_depth * 0.75,
        "far 40 should thin the froxel at 15 m well below far 200: \
         {tight_depth} vs {wide_depth}",
    );
}

/// 🔴 Slice 0 holds everything nearer than the grid starts, so its depth
/// is that whole distance — not what the logarithmic mapping computes.
///
/// A grid starting at 20 m over a scene 10 m away puts every pixel of
/// that scene in one cell, and the panel reported a 0.9 m froxel while
/// the screen turned solid red. The tool has to say 20.
#[test]
fn the_near_slice_holds_everything_before_it() {
    let g = ClusterGrid::new(
        &ClusterSettings {
            first_slice: 20.0,
            far: 60.0,
            ..Default::default()
        },
        Vec2::new(1280.0, 720.0),
    );
    assert_eq!(g.slice_depth(10.0), 20.0);
    assert_eq!(g.slice_depth(0.5), 20.0);
    // Just inside the grid, the mapping applies again.
    assert!(g.slice_depth(25.0) < 20.0);
}

/// And the last slice holds everything behind it, which has no end.
#[test]
fn the_far_slice_is_unbounded() {
    let g = grid();
    assert!(g.slice_depth(10_000.0).is_infinite());
}
