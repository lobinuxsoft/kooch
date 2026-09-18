//! Test code for `slots`, in its own file.

use super::*;

fn colour(size: (u32, u32)) -> TargetDesc {
    TargetDesc::attachment(size, wgpu::TextureFormat::Rgba8Unorm)
}

/// The point of the pool: what one frame released, the next one gets back.
#[test]
fn a_released_slot_comes_back() {
    let mut slots = Slots::default();
    let (first, fresh) = slots.claim(colour((64, 64)));
    assert_eq!(fresh, Fresh::Created);
    slots.release(first);

    let (again, fresh) = slots.claim(colour((64, 64)));
    assert_eq!((again, fresh), (first, Fresh::Reused));
    assert_eq!(slots.len(), 1);
}

/// Two held at once are two targets.
#[test]
fn a_held_slot_is_not_shared() {
    let mut slots = Slots::default();
    let (first, _) = slots.claim(colour((64, 64)));
    let (second, fresh) = slots.claim(colour((64, 64)));
    assert_ne!(second, first);
    assert_eq!(fresh, Fresh::Created);
}

/// A different size is a different target, however many are free.
#[test]
fn another_size_is_another_slot() {
    let mut slots = Slots::default();
    let (first, _) = slots.claim(colour((64, 64)));
    slots.release(first);

    let (second, fresh) = slots.claim(colour((128, 128)));
    assert_ne!(second, first);
    assert_eq!(fresh, Fresh::Created);
}

/// 🔴 #1201's leak: the editor's post-process claims and releases a target every frame and never
/// ends a frame. That must settle at one slot per viewport, not one per frame.
#[test]
fn every_frame_reuses_its_slots() {
    let mut slots = Slots::default();
    for _ in 0..600 {
        let view = slots.claim(colour((1920, 1080))).0;
        let game = slots.claim(colour((1280, 720))).0;
        slots.release(view);
        slots.release(game);
    }
    assert_eq!(slots.len(), 2, "one per viewport, whatever the frame count");
    assert_eq!(slots.free(), 2);
}
