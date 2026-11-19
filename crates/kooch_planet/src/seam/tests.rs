use std::collections::HashSet;

use glam::IVec3;

use super::{FINER, Leaves, OWN, direction, levels, near};
use crate::{BlockKey, BlockPool, Residency};

const ROOT: u8 = 4;

fn key(level: u8, x: i32, y: i32) -> BlockKey {
    BlockKey {
        planet: 0,
        level,
        cell: IVec3::new(x, y, 0),
    }
}

fn side(of: &super::Near, offset: IVec3) -> u32 {
    let d = (0..26).find(|&d| direction(d) == offset).unwrap();
    of.sides[d]
}

/// A level-1 block with a coarser leaf at +X, finer ones at −X, a same-level one at +Y and none at
/// −Y; the coarser tile stops holding once its leaf leaves the pool.
#[test]
fn a_block_sees_its_neighbours() {
    let block = key(1, 1, 0);
    let coarse = key(2, 1, 0);
    let resident = [block, coarse, key(0, 0, 0), key(1, 1, 1)];
    let mut pool = BlockPool::new(8);
    pool.apply(&resident, &mut Residency::default());
    let split: HashSet<BlockKey> = resident.iter().flat_map(|k| k.ancestors(ROOT)).collect();
    let leaves = Leaves {
        pool: &pool,
        split: &split,
    };

    let found = near(leaves, ROOT, block, |_| true);
    let x = side(&found, IVec3::X);
    assert_eq!(levels(x), 1);
    assert_eq!(x & 0xff_ffff, pool.slot(coarse).unwrap());
    assert_eq!(side(&found, IVec3::NEG_X), FINER);
    assert_eq!(side(&found, IVec3::Y), OWN);
    assert_eq!(side(&found, IVec3::NEG_Y), OWN);
    assert!(found.holds(&pool, block));

    let waiting = near(leaves, ROOT, block, |_| false);
    assert_eq!(side(&waiting, IVec3::X), OWN);

    pool.apply(&[block], &mut Residency::default());
    assert!(!found.holds(&pool, block));
}
