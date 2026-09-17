//! Test code for `slots`, in its own file.

use super::*;

fn colour(size: (u32, u32)) -> TargetDesc {
    TargetDesc::attachment(size, wgpu::TextureFormat::Rgba8Unorm)
}

/// Two views of the same size share one slot once the first is released and retired — the point of
/// the pool.
#[test]
fn a_retired_slot_comes_back() {
    let mut slots = Slots::default();
    let (first, fresh) = slots.claim(colour((64, 64)));
    assert_eq!(fresh, Fresh::Created);
    slots.release(first);
    for _ in 0..RETIREMENT {
        slots.end_frame();
    }

    let (again, fresh) = slots.claim(colour((64, 64)));
    assert_eq!((again, fresh), (first, Fresh::Reused));
    assert_eq!(slots.len(), 1);
}

/// 🔴 Mesa radv invalidates a bind group whose texture was dropped in flight, so a slot released
/// this frame must not be handed to anyone until it has waited out its retirement.
#[test]
fn a_fresh_release_is_not_reused() {
    let mut slots = Slots::default();
    let (first, _) = slots.claim(colour((64, 64)));
    slots.release(first);

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
    for _ in 0..RETIREMENT {
        slots.end_frame();
    }

    let (second, fresh) = slots.claim(colour((128, 128)));
    assert_ne!(second, first);
    assert_eq!(fresh, Fresh::Created);
}

/// What #1197 was: resizing over and over must not grow the pool without bound.
#[test]
fn resizing_does_not_grow_forever() {
    let mut slots = Slots::default();
    let mut held = slots.claim(colour((64, 64))).0;
    for _ in 0..64 {
        slots.release(held);
        for _ in 0..RETIREMENT {
            slots.end_frame();
        }
        held = slots.claim(colour((64, 64))).0;
    }
    assert_eq!(slots.len(), 1, "one slot, reused every time");
    assert_eq!(slots.free(), 0);
}
