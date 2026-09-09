use glam::Vec3;

use super::{centre_on, lines, shade_at};

#[test]
fn the_centre_lands_on_the_step() {
    // 🔴 A grid whose lines miss the values the handles snap to is
    // decoration that lies.
    assert_eq!(
        centre_on(Vec3::new(3.3, 0.0, -1.2), 0.5),
        Vec3::new(3.5, 0.0, -1.0)
    );
}

#[test]
fn a_zero_step_centres_where_it_is() {
    // No step means no lattice to land on; refusing to divide beats a
    // NaN that spreads to every line.
    let at = Vec3::new(1.5, 0.0, 2.5);
    assert_eq!(centre_on(at, 0.0), at);
}

#[test]
fn the_centre_is_the_brightest() {
    assert_eq!(shade_at(0.0, 10.0), 1.0);
}

#[test]
fn the_edge_fades_out() {
    assert_eq!(shade_at(10.0, 10.0), 0.0);
    assert_eq!(shade_at(99.0, 10.0), 0.0);
}

#[test]
fn the_far_half_is_already_dim() {
    // Squared, not linear: at halfway a linear ramp still draws a wall
    // of lines behind whatever is being built.
    assert!(shade_at(5.0, 10.0) < 0.3);
}

#[test]
fn a_grid_covers_both_axes() {
    // 81 offsets, two lines each.
    let count = lines(Vec3::ZERO, Vec3::X, Vec3::Z, 1.0).count();
    assert_eq!(count, 81 * 2);
}

#[test]
fn every_tenth_line_counts() {
    let coarse = lines(Vec3::ZERO, Vec3::X, Vec3::Z, 1.0)
        .filter(|line| line.coarse)
        .count();
    // -40, -30 … 40 is nine offsets, two lines each.
    assert_eq!(coarse, 9 * 2);
}

#[test]
fn the_lines_lie_on_their_plane() {
    // A ground grid must not leave the ground: a line drifting in Y is
    // a grid you cannot judge a height against.
    for line in lines(Vec3::ZERO, Vec3::X, Vec3::Z, 0.5) {
        assert_eq!(line.from.y, 0.0);
        assert_eq!(line.to.y, 0.0);
    }
}

#[test]
fn a_tilted_plane_tilts_its_grid() {
    // The guide grid lives on the drag's plane, not on the ground.
    let up = Vec3::Y;
    for line in lines(Vec3::ZERO, Vec3::X, up, 1.0) {
        assert_eq!(line.from.z, 0.0);
        assert_eq!(line.to.z, 0.0);
    }
}
