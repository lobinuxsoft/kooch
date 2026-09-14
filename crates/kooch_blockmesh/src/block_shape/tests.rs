use super::{BlockShape, KIND_CHOICES};
use crate::Shape;

/// 🔴 A remote spawn writes only `kind`, so every default shape has to be the default component
/// with that kind — or the project builds a different shape than the editor wrote.
#[test]
fn defaults_differ_only_in_kind() {
    for (kind, shape) in Shape::DEFAULTS.iter().enumerate() {
        let block = BlockShape::from(*shape);
        let expected = BlockShape {
            kind: kind as u32,
            ..BlockShape::default()
        };
        assert_eq!(block, expected, "{}", shape.label());
        assert_eq!(block.shape(), *shape);
    }
}

#[test]
fn the_dropdown_matches_the_shapes() {
    assert_eq!(KIND_CHOICES.len(), Shape::DEFAULTS.len());
    for (index, choice) in KIND_CHOICES.iter().enumerate() {
        assert_eq!(choice.value, index as i64);
        assert_eq!(choice.label, Shape::DEFAULTS[index].label());
    }
}

#[test]
fn an_unknown_kind_is_a_cube() {
    let block = BlockShape {
        kind: 99,
        ..BlockShape::default()
    };
    assert!(matches!(block.shape(), Shape::Cube { .. }));
}

/// The default pivot is the base: a spawned block stands on the grid.
#[test]
fn the_default_pivot_is_the_base() {
    let mesh = BlockShape::from(Shape::DEFAULTS[1]).build();
    let min_y = mesh
        .positions()
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min);
    assert!(min_y.abs() < 1.0e-4, "base at {min_y}");
}

/// A lower corner as the pivot puts every corner of the box at or above and beside the origin.
#[test]
fn a_corner_pivot_puts_the_box_at_the_origin() {
    let block = BlockShape {
        pivot: glam::Vec3::NEG_ONE,
        ..BlockShape::from(Shape::DEFAULTS[3])
    };
    let mesh = block.build();
    let min = mesh
        .positions()
        .iter()
        .fold(glam::Vec3::INFINITY, |m, p| m.min(*p));
    assert!(min.abs().max_element() < 1.0e-4, "min corner at {min}");
}
