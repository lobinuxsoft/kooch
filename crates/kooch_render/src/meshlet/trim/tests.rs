//! The cut's CPU half (#452): the hull over a coverage, the mesh cut down to it, and when a pair is
//! allowed to bake at all.

use geo::{Area, Contains, Coord, Point};
use glam::Vec2;
use kooch_core::Guid;

use super::{AlphaTrim, HULL_CORNERS, HULL_TRIANGLES, SETTLE_FRAMES, TRIM_SIDE, TrimPair};
use super::{cut, region};
use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::asset::{DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MeshletMesh};
use crate::meshlet::builder::{LodConfig, build_meshlets_lod_chain};

/// The cut's own level: its bake is 0 or 1.
const CLIP: f64 = 0.5;

/// A coverage that keeps the uv square's left half, as a bake would leave it.
fn left_half() -> Vec<u8> {
    covered(|x, _| x < TRIM_SIDE / 2)
}

/// A coverage of every texel `inside` answers for.
fn covered(inside: impl Fn(u32, u32) -> bool) -> Vec<u8> {
    let mut mask = vec![0u8; (TRIM_SIDE * TRIM_SIDE) as usize];
    for y in 0..TRIM_SIDE {
        for x in 0..TRIM_SIDE {
            if inside(x, y) {
                mask[(y * TRIM_SIDE + x) as usize] = 255;
            }
        }
    }
    mask
}

/// A disc of `radius` in uv around the middle of the square: a sprite with empty corners.
fn disc(radius: f32) -> Vec<u8> {
    covered(|x, y| {
        let at = Vec2::new(x as f32, y as f32) / TRIM_SIDE as f32;
        at.distance(Vec2::splat(0.5)) < radius
    })
}

/// A unit quad on the xy plane, its uv spanning the square, `scale`d in uv.
fn quad(scale: f32) -> MeshletMesh {
    let corner = |x: f32, y: f32| MeshVertex {
        position: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        uv: [x * scale, y * scale],
    };
    let mesh = Mesh::from_arrays(
        vec![
            corner(0.0, 0.0),
            corner(1.0, 0.0),
            corner(1.0, 1.0),
            corner(0.0, 1.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    );
    build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("a quad meshletises")
}

/// The quad cut against `mask`'s hull.
fn cut_quad(mask: &[u8], scale: f32) -> Result<cut::Cut, super::NoTrim> {
    let hull = region::hull(mask, TRIM_SIDE, CLIP, HULL_CORNERS).expect("something is covered");
    cut::mesh(&quad(scale), &hull, TRIM_SIDE, CLIP)
}

#[test]
fn a_half_mask_hulls_half() {
    let hull = region::hull(&left_half(), TRIM_SIDE, CLIP, HULL_CORNERS).expect("half is covered");
    let area = hull.region.unsigned_area();
    // Half, and a margin's worth over it: the hull stands off the coverage on purpose.
    assert!(
        (0.5..0.6).contains(&area),
        "half the square hulls to {area}",
    );
}

#[test]
fn an_empty_mask_hulls_nothing() {
    let mask = vec![0u8; (TRIM_SIDE * TRIM_SIDE) as usize];
    assert!(region::hull(&mask, TRIM_SIDE, CLIP, HULL_CORNERS).is_none());
}

/// 🔴 The invariant the whole step rests on: the material still cuts per pixel inside the hull, so a
/// hull that misses one covered texel takes a pixel the alpha wanted, and the mesh is simply wrong.
#[test]
fn a_hull_covers_every_texel() {
    let mask = disc(0.35);
    let hull = region::hull(&mask, TRIM_SIDE, CLIP, HULL_CORNERS).expect("the disc is covered");
    for y in 0..TRIM_SIDE {
        for x in 0..TRIM_SIDE {
            if mask[(y * TRIM_SIDE + x) as usize] < 128 {
                continue;
            }
            let at = Point::from(Coord {
                x: (f64::from(x) + 0.5) / f64::from(TRIM_SIDE),
                y: (f64::from(y) + 0.5) / f64::from(TRIM_SIDE),
            });
            assert!(
                hull.region.contains(&at),
                "texel {x},{y} is outside the hull"
            );
        }
    }
}

/// 🔴 A hull, not a tracing: a disc's edge would take hundreds of segments to follow and a handful
/// to enclose. The corners are what the mesh pays for.
#[test]
fn a_hull_stays_coarse() {
    let hull = region::hull(&disc(0.35), TRIM_SIDE, CLIP, HULL_CORNERS).expect("covered");
    let corners: usize = hull
        .region
        .0
        .iter()
        .map(|polygon| polygon.exterior().0.len())
        .sum();
    assert!(
        corners <= HULL_CORNERS,
        "the disc took {corners} corners, past the budget",
    );
}

/// 🔴 What the cut is for: the quad's empty corners go, and what is left is a handful of triangles
/// rather than a traced outline.
#[test]
fn a_cut_disc_drops_the_corners() {
    let cut = cut_quad(&disc(0.35), 1.0).expect("the disc survives");
    assert!(
        cut.triangles <= HULL_TRIANGLES,
        "{} triangles for a disc",
        cut.triangles,
    );
    // A disc of radius 0.35 covers 0.38 of its square; a hull around it keeps a little more.
    assert!(
        (0.38..0.6).contains(&(cut.kept / cut.whole)),
        "the cut kept {} of {} uv",
        cut.kept,
        cut.whole,
    );
}

/// A uv that tiles reads a coverage that was never baked, so the mesh keeps its own shape.
#[test]
fn a_tiled_uv_is_refused() {
    assert!(matches!(
        cut_quad(&left_half(), 4.0),
        Err(super::NoTrim::Tiled)
    ));
}

/// Nothing covered leaves no triangle, and no mesh to publish.
#[test]
fn an_uncovered_quad_is_dropped() {
    let hull = region::hull(&left_half(), TRIM_SIDE, CLIP, HULL_CORNERS).expect("covered");
    let mut right = quad(1.0);
    for vertex in &mut right.vertices {
        vertex.uv[0] = vertex.uv[0] * 0.2 + 0.8;
    }
    assert!(matches!(
        cut::mesh(&right, &hull, TRIM_SIDE, CLIP),
        Err(super::NoTrim::Empty)
    ));
}

/// 🔴 Mirrored uv winds the other way, and earcut hands back its own winding: a cut piece that kept
/// it would face away and be culled.
#[test]
fn a_mirrored_uv_keeps_facing() {
    let mask = left_half();
    let hull = region::hull(&mask, TRIM_SIDE, CLIP, HULL_CORNERS).expect("covered");
    let mut mirrored = quad(1.0);
    for vertex in &mut mirrored.vertices {
        vertex.uv[0] = 1.0 - vertex.uv[0];
    }
    let cut = cut::mesh(&mirrored, &hull, TRIM_SIDE, CLIP).expect("half the quad survives");
    assert!(
        signed_area(&cut.mesh) < -0.4,
        "the cut faces the other way: {}",
        signed_area(&cut.mesh),
    );
}

/// The uv a cut mesh covers, signed: pieces wound against each other cancel.
fn signed_area(cut: &Mesh) -> f32 {
    cut.indices
        .chunks_exact(3)
        .map(|triangle| {
            let at = |corner: usize| Vec2::from(cut.vertices[triangle[corner] as usize].uv);
            (at(1) - at(0)).perp_dot(at(2) - at(0)) * 0.5
        })
        .sum()
}

/// 🔴 A dragged slider republishes its material every frame; baking each one would read back each
/// one. The pair waits for the values to hold.
#[test]
fn a_moving_material_never_bakes() {
    let mut trim = AlphaTrim::default();
    let (mesh, material) = (Guid::new_v4(), Guid::new_v4());
    let pair = |stamp| {
        vec![TrimPair {
            mesh,
            material,
            slot: 1,
            stamp,
        }]
    };
    for frame in 0..SETTLE_FRAMES * 2 {
        assert!(
            trim.next(&pair(u64::from(frame))).is_none(),
            "frame {frame} baked a material that is still moving",
        );
    }
    for _ in 0..SETTLE_FRAMES {
        trim.next(&pair(7));
    }
    assert!(trim.next(&pair(7)).is_some(), "a settled pair never baked");
}

#[test]
fn a_remembered_pair_is_not_asked() {
    let mut trim = AlphaTrim::default();
    let (mesh, material) = (Guid::new_v4(), Guid::new_v4());
    trim.remember(mesh, material, 7, Some(Guid::new_v4()));
    let pairs = vec![TrimPair {
        mesh,
        material,
        slot: 1,
        stamp: 7,
    }];
    for _ in 0..SETTLE_FRAMES * 2 {
        assert!(trim.next(&pairs).is_none());
    }
    assert!(trim.mesh_for(mesh, material).is_some());
    // Other values are another cut.
    let moved = vec![TrimPair {
        stamp: 8,
        ..pairs[0]
    }];
    for _ in 0..SETTLE_FRAMES {
        trim.next(&moved);
    }
    assert!(trim.next(&moved).is_some());
}
