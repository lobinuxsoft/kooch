//! A frame is a list of views (#592).

mod common;

use common::try_acquire_device;
use kooch_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig};

fn test_stage(device: &wgpu::Device) -> MeshletRenderStage {
    MeshletRenderStage::new(
        device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 4,
            meshlet_capacity: 64,
            ..Default::default()
        },
    )
}

#[test]
fn a_second_view_gets_its_own_cull_buffers() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut stage = test_stage(&device);
    let second = stage.create_view(&device, (32, 32));
    assert_eq!(stage.view_count(), 2);
    assert_ne!(second, stage.primary_view());

    // The point of the whole split: not the same buffer. Sharing these is what makes two
    // overlapping viewports cull each other's geometry away.
    let primary_visible = stage.cull().visible_meshlets_buffer();
    let second_visible = stage
        .view_cull(second)
        .expect("second view is live")
        .visible_meshlets_buffer();
    assert!(!std::ptr::eq(primary_visible, second_visible));
}

#[test]
fn a_view_keeps_its_own_size() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut stage = test_stage(&device);
    let second = stage.create_view(&device, (32, 16));
    assert_eq!(stage.view_size(stage.primary_view()), Some((64, 64)));
    assert_eq!(stage.view_size(second), Some((32, 16)));
}
