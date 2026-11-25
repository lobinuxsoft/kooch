use super::*;

fn sphere(row: &MeshInstance) -> (Vec3, f32) {
    (Vec3::from_array([row.transform[3][0], 0.0, 0.0]), 1.0)
}

fn at(x: f32) -> MeshInstance {
    MeshInstance::new(glam::Mat4::from_translation(Vec3::X * x), 0, 0)
}

#[test]
fn a_still_row_moves_nothing() {
    let mut casters = Casters::default();
    let rows = [at(1.0), at(2.0)];
    casters.update(&rows, &[0, 1], sphere, |_| false);
    let stamp = casters.bounds()[0].hash;
    casters.update(&rows, &[1], sphere, |_| false);
    assert_eq!(casters.bounds()[0].hash, stamp);
    assert_eq!(casters.moved(), &[[2.0, 0.0, 0.0, 1.0]; 2]);
}

#[test]
fn a_dropped_row_moves_its_bounds() {
    let mut casters = Casters::default();
    casters.update(&[at(1.0), at(2.0)], &[0, 1], sphere, |_| false);
    casters.update(&[at(1.0)], &[], sphere, |_| false);
    assert_eq!(casters.moved(), &[[2.0, 0.0, 0.0, 1.0]]);
    assert_eq!(casters.bounds().len(), 1);
}
