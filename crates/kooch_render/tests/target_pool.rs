//! GPU integration test: the target pool hands out real textures and reuses them (#392).

mod common;

use common::try_acquire_device;
use kooch_core::gpu::{RETIREMENT, TargetDesc, TargetPool};

fn colour(size: (u32, u32)) -> TargetDesc {
    TargetDesc::attachment(size, wgpu::TextureFormat::Rgba8Unorm)
}

#[test]
fn a_view_of_the_same_size_is_reused() {
    let Some((device, _queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut pool = TargetPool::default();

    let first = pool.acquire(&device, "first", colour((128, 128)));
    let second = pool.acquire(&device, "second", colour((128, 128)));
    assert_ne!(first, second, "two targets held at once must not be one");
    assert_eq!(pool.created(), 2);

    pool.release(first);
    for _ in 0..RETIREMENT {
        pool.end_frame();
    }
    let third = pool.acquire(&device, "third", colour((128, 128)));
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
    let mut pool = TargetPool::default();

    let mut held = pool.acquire(&device, "view", colour((256, 256)));
    for _ in 0..32 {
        pool.release(held);
        for _ in 0..RETIREMENT {
            pool.end_frame();
        }
        held = pool.acquire(&device, "view", colour((256, 256)));
    }

    assert_eq!(pool.len(), 1, "one texture for thirty-two resizes");
    assert_eq!(pool.bytes(), 256 * 256 * 4);
}
