use super::Shape;
use crate::{Adjacency, BlockMesh};

/// Six times the enclosed volume: positive only when every face winds counter-clockwise from
/// outside, so it checks the winding and the size in one number.
fn six_volume(mesh: &BlockMesh) -> f32 {
    let p = mesh.positions();
    mesh.faces()
        .map(|face| {
            (1..face.len() - 1)
                .map(|k| {
                    let (a, b, c) = (face[0], face[k], face[k + 1]);
                    p[a as usize].dot(p[b as usize].cross(p[c as usize]))
                })
                .sum::<f32>()
        })
        .sum()
}

/// 🔴 Every shape survives `Adjacency::of`: a hole makes every later extrude wrong.
#[test]
fn every_shape_is_closed_and_outward() {
    for shape in Shape::DEFAULTS {
        let mesh = shape.build();
        assert!(Adjacency::of(&mesh).is_closed(), "{} leaks", shape.label());
        assert!(six_volume(&mesh) > 0.0, "{} winds inward", shape.label());
    }
}

/// 🔴 Quads, not triangles: the tool's vocabulary is the face.
#[test]
fn stairs_are_quads() {
    let mesh = Shape::Stairs {
        steps: 3,
        width: 1.0,
        rise: 1.0,
        run: 1.5,
    }
    .build();
    assert!(mesh.faces().all(|face| face.len() >= 4));
    assert_eq!(mesh.face_count(), 3 * 5 + 1);
}

/// Two steps of width 1, rise 1 and run 1 are columns 0.5 and 1.0 tall, each 0.5 deep: 0.75 m³.
#[test]
fn stairs_hold_their_volume() {
    let mesh = Shape::Stairs {
        steps: 2,
        width: 1.0,
        rise: 1.0,
        run: 1.0,
    }
    .build();
    assert!((six_volume(&mesh) / 6.0 - 0.75).abs() < 1.0e-4);
}

/// A parameter dragged to zero or below still builds something closed.
#[test]
fn degenerate_parameters_still_close() {
    let shapes = [
        Shape::Stairs {
            steps: 0,
            width: 0.0,
            rise: -1.0,
            run: 0.0,
        },
        Shape::Arch {
            segments: 0,
            radius: 0.0,
            thickness: 0.0,
            depth: 0.0,
        },
        Shape::Cylinder {
            sides: 1,
            radius: 0.0,
            height: 0.0,
        },
        Shape::Cone {
            sides: 2,
            radius: -1.0,
            height: 0.0,
        },
        Shape::Plane {
            subdivisions: 0,
            size: 0.0,
            thickness: 0.0,
        },
    ];
    for shape in shapes {
        let mesh = shape.build();
        assert!(Adjacency::of(&mesh).is_closed(), "{} leaks", shape.label());
    }
}
