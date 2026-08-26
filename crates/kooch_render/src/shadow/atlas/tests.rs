use super::*;

use glam::Mat4;

/// A cascade with nothing meaningful in it: these tests are about which
/// layer each one is assigned, and the placement is irrelevant to that.
fn placeholder() -> Cascade {
    Cascade {
        view_proj: Mat4::IDENTITY,
        light_eye: glam::Vec3::ZERO,
        far_depth: 0.0,
        texel_world_size: 1.0,
        depth_extent: 1.0,
    }
}

/// The quadrant packing these tests used to check is gone with the
/// atlas. What replaces it is the one thing a layer layout can still get
/// wrong: two cascades naming the same layer, which samples one
/// cascade's depths through another's transform and reads as a shadow
/// from the wrong distance — the exact failure the packing tests
/// existed to prevent.
#[test]
fn every_cascade_gets_its_own_layer() {
    let cascades = [placeholder(); CASCADE_COUNT];
    let gpu = gpu_cascade_layers(&cascades);

    let mut seen = Vec::new();
    for (i, cascade) in gpu.iter().enumerate() {
        assert!(
            !seen.contains(&cascade.layer),
            "cascade {i} reuses layer {}",
            cascade.layer,
        );
        seen.push(cascade.layer);
    }
    assert_eq!(seen.len(), CASCADE_COUNT);
}

/// The layer is an index into the texture's layers, so it has to stay
/// inside the count the texture was allocated with. One past the end is
/// not a validation error at sample time — it clamps, and clamping means
/// every cascade past the end silently shares the last one's depths.
#[test]
fn no_layer_points_past_the_texture() {
    let gpu = gpu_cascade_layers(&[placeholder(); CASCADE_COUNT]);
    for cascade in &gpu {
        assert!((cascade.layer as usize) < CASCADE_COUNT);
    }
}

/// Four layers of `size²`, not one texture of `2·size` square. The pixel
/// count is identical — which is the point, the migration off the atlas
/// was not supposed to cost or save memory — so a regression here means
/// somebody changed the layer count without saying so.
#[test]
fn the_array_costs_what_the_atlas_did() {
    let size: u64 = 2048;
    let per_layer = size * size * 4;
    assert_eq!(per_layer * CASCADE_COUNT as u64, 4096 * 4096 * 4);
}
