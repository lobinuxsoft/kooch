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
        Stamp {
            shape: SHAPE_CAPSULE,
            position: Vec3::new(-2.0, 51.0, 2.0),
            radius: 1.5,
            height: 6.0,
            ..Stamp::default()
        },
        Stamp {
            shape: SHAPE_TORUS,
            operation: OPERATION_SUBTRACT,
            position: Vec3::new(1.0, 53.0, -2.0),
            radius: 3.0,
            tube: 1.0,
            ..Stamp::default()
        },
        Stamp {
            shape: SHAPE_CONE,
            position: Vec3::new(2.0, 54.0, 2.0),
            radius: 3.0,
            height: 5.0,
            turn: -40.0,
            ..Stamp::default()
        },
    ];
    Stack::new(&stamps, (&[], &[]), Base::Sphere, 0.5, 0)
}

/// The range must hold every point of its box, or the selection prunes a block that has ground.
#[test]
fn the_bound_holds_every_point() {
    holds(&stack(), None);
}

/// A volume is bounded by its own range, not by distance: a graph's SDF need not be one.
#[test]
fn a_volume_bound_holds() {
    let volume = "fn volume(input: PlanetInput) -> f32 {\n    \
        return (length(input.position - vec3<f32>(0.0, 1.0, 0.0)) - 3.0) * 2.5 + sin(input.position.x * 2.0);\n}\n";
    let source = crate::join_volumes(crate::BARE_SPHERE, &[volume]).unwrap();
    let generator = Generator::parse(&source).unwrap();
    let guid = kooch_core::Guid::from_bytes([7; 16]);
    let stamps = [Stamp {
        shape: SHAPE_VOLUME,
        volume: Some(guid),
        position: Vec3::new(0.0, 51.0, 1.0),
        radius: 6.0,
        smooth: 1.0,
        turn: 20.0,
        ..Stamp::default()
    }];
    let stack = Stack::new(&stamps, (&[guid], &[]), Base::Sphere, 0.5, 0);
    holds(&stack, Some((&generator, 0)));
}

/// A field stamp too: its gain keeps a steep map a distance, which is all the bound assumes.
#[test]
fn a_field_bound_holds() {
    let texels: Vec<u16> = (0..64u16).map(|i| (i * 1031) % 65535).collect();
    let field = crate::Field {
        kind: crate::FIELD_HEIGHT,
        size: [8, 1, 8],
        extent: [1.0; 3],
        lo: 0.0,
        hi: 1.0,
        slope: 1.0,
        texels: texels.into(),
    };
    let guid = kooch_core::Guid::from_bytes([9; 16]);
    let stamps = [Stamp {
        shape: SHAPE_FIELD,
        field: Some(guid),
        position: Vec3::new(1.0, 51.0, -1.0),
        size: Vec3::new(5.0, 3.0, 4.0),
        turn: 15.0,
        ..Stamp::default()
    }];
    let fields = [(guid, std::sync::Arc::new(field))];
    let stack = Stack::new(&stamps, (&[], &fields), Base::Sphere, 0.5, 0);
    holds(&stack, None);
}

fn holds(stack: &Stack, joined: Joined<'_>) {
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
        let (lo, hi) = stack.bound((min, max), range, joined);
        for i in 0..=8 {
            for j in 0..=8 {
                for k in 0..=8 {
                    let t = DVec3::new(f64::from(i), f64::from(j), f64::from(k)) / 8.0;
                    let p = min + (max - min) * t;
                    let d = stack.apply(p, ground(p), joined);
                    assert!(lo <= d && d <= hi, "{d} outside {lo}..{hi} at {p}");
                }
            }
        }
    }
}
