use super::*;

#[test]
fn faces_of_one_light_are_contiguous() {
    // `textureSampleCompareLevel` on a cube array takes the light index
    // and derives the face from the direction, so the six faces of light
    // `n` have to be layers 6n..6n+6. Interleaving lights would sample
    // another lamp's ceiling.
    for slot in 0..4 {
        let base = PointShadowCubes::layer(slot, 0);
        for face in 0..CUBE_FACES {
            assert_eq!(PointShadowCubes::layer(slot, face), base + face as u32);
        }
    }
    assert_eq!(PointShadowCubes::layer(1, 0), CUBE_FACES as u32);
}

#[test]
fn the_budget_is_six_mib_per_light() {
    // The number `MAX_POINT_SHADOWS` was chosen against. If this moves,
    // that constant's reasoning moved with it.
    let bytes = (DEFAULT_CUBE_SIZE as u64) * (DEFAULT_CUBE_SIZE as u64) * 4 * CUBE_FACES as u64;
    assert_eq!(bytes, 6 * 1024 * 1024);
}
