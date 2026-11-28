use glam::{DVec3, Vec3};

use super::*;

fn stack() -> Stack {
    let stamps = [
        Stamp {
            position: Vec3::new(0.0, 52.0, 0.0),
            radius: 6.0,
            smooth: 2.0,
            ..Stamp::default()
        },
        Stamp {
            shape: SHAPE_BOX,
            operation: OPERATION_SUBTRACT,
            position: Vec3::new(3.0, 50.0, 1.0),
            size: Vec3::new(2.0, 4.0, 3.0),
            turn: 30.0,
            smooth: 1.0,
            opacity: 0.7,
            ..Stamp::default()
        },
    ];
    Stack::new(&stamps, Base::Sphere, 0.5, 0)
}

/// The range must hold every point of its box, or the selection prunes a block that has ground.
#[test]
fn the_bound_holds_every_point() {
    let stack = stack();
    let ground = |p: DVec3| p.length() - 50.0;
    for corner in [
        DVec3::new(-4.0, 44.0, -4.0),
        DVec3::new(0.0, 50.0, 0.0),
        DVec3::new(2.0, 55.0, -3.0),
    ] {
        let (min, max) = (corner, corner + DVec3::splat(4.0));
        let reach = (max - min).length() * 0.5;
        let centre = (min + max) * 0.5;
        let range = (ground(centre) - reach, ground(centre) + reach);
        let (lo, hi) = stack.bound(min, max, range);
        for i in 0..=8 {
            for j in 0..=8 {
                for k in 0..=8 {
                    let t = DVec3::new(f64::from(i), f64::from(j), f64::from(k)) / 8.0;
                    let p = min + (max - min) * t;
                    let d = stack.apply(p, ground(p));
                    assert!(lo <= d && d <= hi, "{d} outside {lo}..{hi} at {p}");
                }
            }
        }
    }
}
