//! GPU integration test: the target pool hands out real textures and reuses them (#392).

mod common;

use common::try_acquire_device;
use kooch_core::gpu::{TargetDesc, TargetPool};

fn colour(size: (u32, u32)) -> TargetDesc {
    TargetDesc::attachment(size, wgpu::TextureFormat::Rgba8Unorm)
}

#[test]
fn a_view_of_the_same_size_is_reused() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut pool = TargetPool::new(&device);

    let first = pool.acquire("first", colour((128, 128)));
    let second = pool.acquire("second", colour((128, 128)));
    assert_ne!(first, second, "two targets held at once must not be one");
    assert_eq!(pool.created(), 2);

    pool.release(first);
    let third = pool.acquire("third", colour((128, 128)));
    assert_eq!(third, first, "the released target came back");
    assert_eq!(pool.created(), 2, "nothing new was allocated");
    assert!(pool.view(third).is_some());
}

/// The VRAM acceptance of #392: resizing a view again and again settles at one target rather than
/// one per resize.
#[test]
fn resizing_settles_at_one_target() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut pool = TargetPool::new(&device);

    let mut held = pool.acquire("view", colour((256, 256)));
    for _ in 0..32 {
        pool.release(held);
        held = pool.acquire("view", colour((256, 256)));
    }

    assert_eq!(pool.len(), 1, "one texture for thirty-two resizes");
    assert_eq!(pool.bytes(), 256 * 256 * 4);
}

/// A view dragged through many sizes keeps the targets of the last few, not one per size.
#[test]
fn a_dragged_view_frees_old_sizes() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut pool = TargetPool::new(&device);

    for width in (256..512).step_by(8) {
        let held = pool.acquire("view", colour((width, 256)));
        pool.release(held);
        pool.end_frame();
    }

    assert!(pool.len() <= 4, "{} textures for 32 sizes", pool.len());
    let last = pool.acquire("view", colour((504, 256)));
    assert!(
        pool.view(last).is_some(),
        "the size in use kept its texture"
    );
}
