use super::*;
use kooch_lighting::LIGHT_KIND_POINT;

fn lamp(at: Vec3, range: f32) -> GpuLight {
    GpuLight {
        position: at.into(),
        range,
        kind: LIGHT_KIND_POINT,
        ..bytemuck::Zeroable::zeroed()
    }
}

/// A distant moving lamp keeps its drawn pose between redraws, and catches up on its frame.
#[test]
fn a_distant_lamp_lags() {
    let eye = Vec3::ZERO;
    let mut posed = Vec::new();
    pace(&mut posed, &[lamp(Vec3::new(100.0, 0.0, 0.0), 1.0)], eye, 0);
    let mut moved = Vec::new();
    for frame in 1..=4 {
        let now = lamp(Vec3::new(100.0 + frame as f32, 0.0, 0.0), 1.0);
        pace(&mut posed, &[now], eye, frame);
        moved.push(posed[0].position == now.position);
    }
    assert_eq!(moved, [false, false, false, true]);
}

/// A lamp near the eye redraws every frame.
#[test]
fn a_near_lamp_keeps_up() {
    let mut posed = Vec::new();
    for frame in 0..3 {
        let now = lamp(Vec3::new(1.0 + frame as f32, 0.0, 0.0), 5.0);
        pace(&mut posed, &[now], Vec3::ZERO, frame);
        assert_eq!(posed[0].position, now.position);
    }
}

/// A change other than the pose is taken at once, whatever the interval.
#[test]
fn a_range_change_is_immediate() {
    let mut posed = Vec::new();
    let far = Vec3::new(100.0, 0.0, 0.0);
    pace(&mut posed, &[lamp(far, 1.0)], Vec3::ZERO, 0);
    pace(&mut posed, &[lamp(far, 2.0)], Vec3::ZERO, 1);
    assert_eq!(posed[0].range, 2.0);
}
