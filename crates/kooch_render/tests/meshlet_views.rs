//! A frame is a list of views (#592).
//!
//! Pins the property the split exists for: a second view gets its own
//! cull output, so two cameras cannot overwrite each other's survivor
//! list mid-frame. That is the shape of `bevyengine/bevy#15182`, where
//! overlapping viewports over-cull because per-frame visibility state
//! is shared across views.
//!
//! Run with:
//!   cargo test -p kooch_render --test meshlet_views

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
fn a_new_stage_has_exactly_one_view() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let stage = test_stage(&device);
    assert_eq!(stage.view_count(), 1);
    assert!(stage.has_view(stage.primary_view()));
}

#[test]
fn a_second_view_gets_its_own_cull_buffers() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut stage = test_stage(&device);
    let config = MeshletRenderStageConfig {
        size: (64, 64),
        instance_capacity: 4,
        meshlet_capacity: 64,
        ..Default::default()
    };

    let second = stage.create_view(&device, (32, 32), &config);
    assert_eq!(stage.view_count(), 2);
    assert_ne!(second, stage.primary_view());

    // The point of the whole split: not the same buffer. Sharing these
    // is what makes two overlapping viewports cull each other's
    // geometry away.
    //
    // Identity, not contents: before the split both views resolved to
    // one field of one struct, so this comparison was `true` by
    // construction. It cannot tell two distinct buffers holding equal
    // bytes apart, which is fine — that is not the failure mode.
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
    let config = MeshletRenderStageConfig {
        size: (64, 64),
        instance_capacity: 4,
        meshlet_capacity: 64,
        ..Default::default()
    };

    let second = stage.create_view(&device, (32, 16), &config);
    assert_eq!(stage.view_size(stage.primary_view()), Some((64, 64)));
    assert_eq!(stage.view_size(second), Some((32, 16)));
}

#[test]
fn a_destroyed_view_stops_resolving() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut stage = test_stage(&device);
    let config = MeshletRenderStageConfig {
        size: (64, 64),
        instance_capacity: 4,
        meshlet_capacity: 64,
        ..Default::default()
    };

    let second = stage.create_view(&device, (32, 32), &config);
    assert!(stage.destroy_view(second));
    assert_eq!(stage.view_count(), 1);

    // Generational key: the handle reads as gone, not as whichever view
    // lands in that slot next. A closed editor panel leaves its id
    // behind, and a bare index would silently address a stranger.
    assert!(!stage.has_view(second));
    assert_eq!(stage.view_size(second), None);
    assert!(stage.view_cull(second).is_none());

    let third = stage.create_view(&device, (8, 8), &config);
    assert_ne!(third, second);
    assert!(!stage.has_view(second));
}

#[test]
fn the_primary_view_cannot_be_destroyed() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let mut stage = test_stage(&device);
    // A stage with no view cannot render, and every single-view
    // accessor would have to start returning Option.
    assert!(!stage.destroy_view(stage.primary_view()));
    assert_eq!(stage.view_count(), 1);
}
