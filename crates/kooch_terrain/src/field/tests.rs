use glam::DVec3;

use super::*;

/// A steep 8×8 map: every other texel at the top.
pub(crate) fn spikes() -> Field {
    let texels: Vec<u16> = (0..64u16)
        .map(|i| if (i + i / 8) % 2 == 0 { 65535 } else { 0 })
        .collect();
    Field {
        kind: FIELD_HEIGHT,
        size: [8, 1, 8],
        extent: [1.0; 3],
        lo: 0.0,
        hi: 1.0,
        slope: 1.0,
        texels: texels.into(),
    }
}

#[test]
fn a_field_survives_its_file() {
    let field = spikes();
    assert_eq!(Field::from_bytes(&field.to_bytes()).unwrap(), field);
}

/// The stack bounds a stamp by its distance at a box's centre, so the field must change by no
/// more than a metre per metre, however steep the map or squashed the stamp.
#[test]
fn a_steep_field_stays_lipschitz() {
    let field = spikes();
    let half = DVec3::new(4.0, 6.0, 4.0);
    let gain = field.gain(half);
    let step = 0.05;
    for i in 0..400 {
        let p = DVec3::new(
            (i as f64 * 0.37).sin() * 5.0,
            (i as f64 * 0.11).cos() * 7.0,
            (i as f64 * 0.23).sin() * 5.0,
        );
        let a = distance(&field, p, half, gain);
        for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
            let b = distance(&field, p + axis * step, half, gain);
            assert!((a - b).abs() <= step * 1.0001, "{a} → {b} at {p}");
        }
    }
}
