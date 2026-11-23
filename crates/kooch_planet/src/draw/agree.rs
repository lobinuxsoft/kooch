//! Whether two meshes agree on the seam between them. Each mesh reads who holds the cells past it
//! from the `Near` it was counted with; two that read it differently leave a crack (#1979), so the
//! swap shows a mesh only when it agrees with every neighbour on screen with it.

use glam::IVec3;

use crate::seam::{FINER, OWN, direction, levels, slot};
use crate::{BlockKey, BlockPool, Near};

/// What a block will have on screen after the swap.
#[derive(Clone, Copy)]
pub(super) enum Shows {
    Mesh(Near),
    /// Nothing to draw: no surface, so no seam to disagree on.
    Empty,
    /// Not meshed yet, or held back: whatever is drawn there is not this block.
    Missing,
}

/// Whether `key`'s mesh, counted with `near`, agrees with the pool's leaves around it.
pub(super) fn agrees(
    pool: &BlockPool,
    root: u8,
    key: BlockKey,
    near: &Near,
    shows: impl Fn(BlockKey) -> Shows,
) -> bool {
    (0..26).all(|d| side_agrees(pool, root, key, d, near.sides[d], &shows))
}

fn side_agrees(
    pool: &BlockPool,
    root: u8,
    key: BlockKey,
    d: usize,
    side: u32,
    shows: &impl Fn(BlockKey) -> Shows,
) -> bool {
    let offset = direction(d);
    let across = BlockKey {
        cell: key.cell + offset,
        ..key
    };
    // Same level: both own their side of it.
    if pool.slot(across).is_some() {
        return side == OWN && facing(shows(across), |n| n.sides[25 - d] == OWN);
    }
    // Coarser: this one borrows its tile, and it leaves the seam to the finer side.
    let coarser = across
        .ancestors(root)
        .zip(key.ancestors(root))
        .take_while(|(up, ours)| up != ours)
        .find(|(up, _)| pool.slot(*up).is_some());
    if let Some((up, ours)) = coarser {
        let held =
            levels(side) == u32::from(up.level - key.level) && pool.key(slot(side)) == Some(up);
        return held
            && facing(shows(up), |n| {
                index_of(ours.cell - up.cell).is_some_and(|i| n.sides[i] == FINER)
            });
    }
    // Finer: each touching child borrows this block's tile.
    let Some(ours) = pool.slot(key) else {
        return false;
    };
    let mut finer = touching(across.cell, offset).map(|cell| BlockKey {
        level: key.level.wrapping_sub(1),
        cell,
        ..key
    });
    let mut any = false;
    let agreed = finer.all(|child| {
        if key.level == 0 || pool.slot(child).is_none() {
            return true;
        }
        any = true;
        facing(shows(child), |n| borrows_from(n, child, key, ours))
    });
    agreed && (!any || side == FINER)
}

/// A neighbour agrees when it draws nothing, and never when it is missing.
fn facing(shows: Shows, check: impl Fn(&Near) -> bool) -> bool {
    match shows {
        Shows::Mesh(near) => check(&near),
        Shows::Empty => true,
        Shows::Missing => false,
    }
}

/// Whether every side of `child` landing in `coarse` borrows its tile from `slot_of`.
fn borrows_from(near: &Near, child: BlockKey, coarse: BlockKey, slot_of: u32) -> bool {
    near.sides.iter().enumerate().all(|(d, &side)| {
        let lands = (child.cell + direction(d)).div_euclid(IVec3::splat(2)) == coarse.cell;
        !lands || (levels(side) == 1 && slot(side) == slot_of)
    })
}

/// The children of `across` touching the block it lies `offset` past.
fn touching(across: IVec3, offset: IVec3) -> impl Iterator<Item = IVec3> {
    (0..8).filter_map(move |i| {
        let bit = IVec3::new(i & 1, (i >> 1) & 1, (i >> 2) & 1);
        let near_side = IVec3::select(offset.cmpgt(IVec3::ZERO), IVec3::ZERO, IVec3::ONE);
        let fits = (offset.cmpeq(IVec3::ZERO) | bit.cmpeq(near_side)).all();
        fits.then_some(across * 2 + bit)
    })
}

/// The index of a unit offset among the 26 directions.
fn index_of(offset: IVec3) -> Option<usize> {
    if offset == IVec3::ZERO || offset.abs().max_element() > 1 {
        return None;
    }
    let full = (offset + IVec3::ONE).dot(IVec3::new(1, 3, 9)) as usize;
    Some(full - usize::from(full > 13))
}
