use super::*;

#[test]
fn quadrants_tile_the_atlas_without_overlapping() {
    let regions = quadrants(2048);
    for (i, a) in regions.iter().enumerate() {
        for b in regions.iter().skip(i + 1) {
            let disjoint = a.x + a.size <= b.x
                || b.x + b.size <= a.x
                || a.y + a.size <= b.y
                || b.y + b.size <= a.y;
            assert!(disjoint, "cascades overlap: {a:?} and {b:?}");
        }
    }
    // And they cover it: four quadrants of a 2× square.
    let covered: u32 = regions.iter().map(|r| r.size * r.size / 1024).sum();
    assert_eq!(covered, (4096 * 4096) / 1024);
}

#[test]
fn the_near_cascade_is_top_left() {
    let regions = quadrants(2048);
    assert_eq!(
        regions[0],
        AtlasRegion {
            x: 0,
            y: 0,
            size: 2048
        }
    );
}

/// The uv transform has to land a cascade's full `[0,1]` inside its
/// own quadrant and nowhere else. Getting the bias wrong samples a
/// neighbouring cascade, which looks like a shadow from the wrong
/// distance rather than like a broken transform.
#[test]
fn uv_transform_maps_each_cascade_into_its_own_quadrant() {
    let regions = quadrants(2048);
    let atlas = 4096;
    for (i, region) in regions.iter().enumerate() {
        let [sx, sy, bx, by] = region.uv_scale_bias(atlas);
        let corner = |u: f32, v: f32| (u * sx + bx, v * sy + by);

        let (x0, y0) = corner(0.0, 0.0);
        let (x1, y1) = corner(1.0, 1.0);
        let expect_x0 = region.x as f32 / atlas as f32;
        let expect_y0 = region.y as f32 / atlas as f32;

        assert!((x0 - expect_x0).abs() < 1e-6, "cascade {i} u origin");
        assert!((y0 - expect_y0).abs() < 1e-6, "cascade {i} v origin");
        assert!(
            (x1 - (expect_x0 + 0.5)).abs() < 1e-6,
            "cascade {i} must span exactly half the atlas, got {x1}",
        );
        assert!(
            (y1 - (expect_y0 + 0.5)).abs() < 1e-6,
            "cascade {i} v extent"
        );
        assert!((0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&y1));
    }
}

#[test]
fn the_atlas_is_twice_a_cascade_on_each_axis() {
    let regions = quadrants(1024);
    let max_x = regions.iter().map(|r| r.x + r.size).max().unwrap();
    let max_y = regions.iter().map(|r| r.y + r.size).max().unwrap();
    assert_eq!((max_x, max_y), (2048, 2048));
}
