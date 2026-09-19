//! The cut's CPU half (#452): the contour over a mask, the mesh cut against it, and when a pair is
//! allowed to bake at all.

use glam::Vec2;
use kooch_core::Guid;

use super::{AlphaTrim, SETTLE_FRAMES, TRIM_SIDE, TrimPair, cut, region};
use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::asset::{DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MeshletMesh};
use crate::meshlet::builder::{LodConfig, build_meshlets_lod_chain};

/// A cut that keeps the uv square's left half, as a bake would leave it.
fn left_half() -> Vec<u8> {
    let mut mask = vec![0u8; (TRIM_SIDE * TRIM_SIDE) as usize];
    for y in 0..TRIM_SIDE {
        for x in 0..TRIM_SIDE / 2 {
            mask[(y * TRIM_SIDE + x) as usize] = 255;
        }
    }
    mask
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

#[test]
fn a_half_mask_contours_half() {
    let coverage = region::coverage(&left_half(), TRIM_SIDE).expect("half a square is covered");
    let area = geo::Area::unsigned_area(&coverage);
    assert!(
        (area - 0.5).abs() < 0.01,
        "half the square is {area}, not 0.5",
    );
}

#[test]
fn an_empty_mask_contours_nothing() {
    let mask = vec![0u8; (TRIM_SIDE * TRIM_SIDE) as usize];
    assert!(region::coverage(&mask, TRIM_SIDE).is_none());
}

/// 🔴 The point of the whole step: the geometry ends where the alpha does.
#[test]
fn a_cut_quad_loses_half() {
    let mask = left_half();
    let coverage = region::coverage(&mask, TRIM_SIDE).expect("half a square is covered");
    let cut = cut::mesh(&quad(1.0), &coverage, &mask, TRIM_SIDE).expect("half the quad survives");
    let far = cut
        .vertices
        .iter()
        .map(|vertex| vertex.uv[0])
        .fold(0.0f32, f32::max);
    assert!(far < 0.51, "the cut mesh reaches uv {far}, past the alpha");
    let area = signed_area(&cut);
    // Signed, and the source quad winds one way: a flipped piece would cancel instead of add.
    assert!(
        (area - 0.5).abs() < 0.01,
        "the cut covers {area} of the quad's uv, not half",
    );
}

/// 🔴 Mirrored uv winds the other way, and earcut hands back its own winding: a cut piece that kept
/// it would face away and be culled.
#[test]
fn a_mirrored_uv_keeps_facing() {
    let mask = left_half();
    let coverage = region::coverage(&mask, TRIM_SIDE).expect("half a square is covered");
    let mut mirrored = quad(1.0);
    for vertex in &mut mirrored.vertices {
        vertex.uv[0] = 1.0 - vertex.uv[0];
    }
    let cut = cut::mesh(&mirrored, &coverage, &mask, TRIM_SIDE).expect("half the quad survives");
    assert!(
        signed_area(&cut) < -0.4,
        "the cut faces the other way: {}",
        signed_area(&cut),
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

/// A uv that tiles reads a coverage that was never baked, so the mesh keeps its masked raster.
#[test]
fn a_tiled_uv_is_refused() {
    let mask = left_half();
    let coverage = region::coverage(&mask, TRIM_SIDE).expect("half a square is covered");
    assert!(cut::mesh(&quad(4.0), &coverage, &mask, TRIM_SIDE).is_none());
}

/// Nothing covered leaves no triangle, and no mesh to publish.
#[test]
fn an_uncovered_quad_is_dropped() {
    let mask = left_half();
    let coverage = region::coverage(&mask, TRIM_SIDE).expect("half a square is covered");
    let right = {
        let mut mesh = quad(1.0);
        for vertex in &mut mesh.vertices {
            vertex.uv[0] = vertex.uv[0] * 0.4 + 0.6;
        }
        mesh
    };
    assert!(cut::mesh(&right, &coverage, &mask, TRIM_SIDE).is_none());
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
