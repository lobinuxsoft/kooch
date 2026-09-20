//! The mesh cut against the hull, triangle by triangle in uv space (#452). What falls outside it is
//! dropped, what straddles it is clipped and triangulated again, and what is whole keeps the
//! vertices it came with. The hull stands off the coverage, so the material still cuts and blends
//! per pixel inside — what this saves is the fill of everything the alpha never reached.

use std::collections::HashMap;

use geo::{Area, BooleanOps, LineString, MultiPolygon, Polygon, TriangulateEarcut};
use glam::{Vec2, Vec3};

use super::NoTrim;
use super::region::{Cover, Hull, covers};
use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::asset::MeshletMesh;

/// Uv this far outside the square still reads the bake. Further out it tiles, and a tiled coverage
/// is not the one square that was baked, so the mesh is left to the masked raster.
const UV_SLACK: f32 = 0.001;
/// Uv area under which a clipped piece is a sliver the raster would never fill.
const AREA_FLOOR: f64 = 1e-10;

/// What a cut mesh is worth: the mesh, and how much of the source's uv it still covers.
pub(super) struct Cut {
    pub mesh: Mesh,
    pub triangles: usize,
    /// Uv the cut keeps, against the uv the source covered. The fill saved, as a share.
    pub kept: f32,
    pub whole: f32,
}

/// `source`'s full detail cut against `hull`.
pub(super) fn mesh(
    source: &MeshletMesh,
    hull: &Hull,
    side: u32,
    threshold: f64,
) -> Result<Cut, NoTrim> {
    let triangles = lod0(source).ok_or(NoTrim::Empty)?;
    let outside = source.vertices.iter().any(|vertex| {
        let uv = Vec2::from(vertex.uv);
        uv.min_element() < -UV_SLACK || uv.max_element() > 1.0 + UV_SLACK
    });
    if outside {
        return Err(NoTrim::Tiled);
    }
    let mut weld = Weld::default();
    let mut whole = 0.0f32;
    for triangle in triangles {
        let corners = triangle.map(|at| source.vertices[at as usize]);
        let uv = corners.map(|corner| Vec2::from(corner.uv));
        whole += area_of(&uv);
        let (min, max) = (uv[0].min(uv[1]).min(uv[2]), uv[0].max(uv[1]).max(uv[2]));
        // Against the GROWN mask: a triangle inside the hull's margin but past the alpha still has
        // to be kept, or the cut would take back what the margin exists to leave.
        match covers(&hull.grown, side, threshold, min, max) {
            Cover::None => {}
            Cover::All => weld.push(corners),
            Cover::Edge => clip(&corners, &uv, &hull.region, &mut weld),
        }
    }
    if weld.indices.len() < 3 {
        return Err(NoTrim::Empty);
    }
    let kept = weld
        .indices
        .chunks_exact(3)
        .map(|triangle| {
            let uv =
                [0, 1, 2].map(|corner| Vec2::from(weld.vertices[triangle[corner] as usize].uv));
            area_of(&uv)
        })
        .sum();
    Ok(Cut {
        triangles: weld.indices.len() / 3,
        mesh: Mesh::from_arrays(weld.vertices, weld.indices),
        kept,
        whole,
    })
}

/// A triangle's uv area, unsigned.
fn area_of(uv: &[Vec2; 3]) -> f32 {
    ((uv[1] - uv[0]).perp_dot(uv[2] - uv[0]) * 0.5).abs()
}

/// The full-detail triangles as indices into [`MeshletMesh::vertices`]. The coarser levels are
/// rebuilt from them, so only level 0 is cut.
fn lod0(source: &MeshletMesh) -> Option<Vec<[u32; 3]>> {
    let mut out = Vec::new();
    for meshlet in source.meshlets.iter().filter(|m| m.lod_level == 0) {
        let base = meshlet.vertex_offset as usize;
        for triangle in 0..meshlet.triangle_count as usize {
            let at = meshlet.triangle_offset as usize + triangle * 3;
            let corner = |corner: usize| {
                let local = source.meshlet_triangles[at + corner] as usize;
                source.meshlet_vertices[base + local]
            };
            out.push([corner(0), corner(1), corner(2)]);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Cuts one triangle against `coverage` and welds what is left, each piece wound as the triangle it
/// came from — the masked geometry is back-face culled, and earcut has no winding to preserve.
fn clip(corners: &[MeshVertex; 3], uv: &[Vec2; 3], coverage: &MultiPolygon<f64>, weld: &mut Weld) {
    let point = |at: Vec2| (f64::from(at.x), f64::from(at.y));
    let ring = LineString::from(vec![point(uv[0]), point(uv[1]), point(uv[2]), point(uv[0])]);
    let triangle = MultiPolygon::new(vec![Polygon::new(ring, Vec::new())]);
    let turn = (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]);
    for polygon in triangle.intersection(coverage).0.iter() {
        for piece in polygon.earcut_triangles_iter() {
            if piece.unsigned_area() < AREA_FLOOR {
                continue;
            }
            let at = piece.to_array();
            let mut cut = [0usize, 1, 2].map(|corner| {
                let point = Vec2::new(at[corner].x as f32, at[corner].y as f32);
                interpolate(corners, uv, point)
            });
            let piece_turn = {
                let to = |corner: usize| Vec2::new(at[corner].x as f32, at[corner].y as f32);
                (to(1) - to(0)).perp_dot(to(2) - to(0))
            };
            if piece_turn.is_sign_negative() != turn.is_sign_negative() {
                cut.swap(1, 2);
            }
            weld.push(cut);
        }
    }
}

/// The vertex a uv point stands for, read across the triangle it lies in.
fn interpolate(corners: &[MeshVertex; 3], uv: &[Vec2; 3], point: Vec2) -> MeshVertex {
    let area = (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]);
    if area.abs() < f32::EPSILON {
        return corners[0];
    }
    let weights = [
        (uv[1] - point).perp_dot(uv[2] - point) / area,
        (uv[2] - point).perp_dot(uv[0] - point) / area,
        (uv[0] - point).perp_dot(uv[1] - point) / area,
    ];
    let mut position = Vec3::ZERO;
    let mut normal = Vec3::ZERO;
    for (corner, weight) in corners.iter().zip(weights) {
        position += Vec3::from(corner.position) * weight;
        normal += Vec3::from(corner.normal) * weight;
    }
    MeshVertex {
        position: position.to_array(),
        normal: normal.normalize_or_zero().to_array(),
        // The point itself, not a weighted uv: the cut is exact in uv and rounding it would move
        // the edge the whole trim exists to place.
        uv: point.to_array(),
    }
}

/// The cut mesh as it is built: vertices that match bit for bit share an index, so the LOD chain
/// rebuilt from it can still collapse across a triangle's edges.
#[derive(Default)]
struct Weld {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
    seen: HashMap<[u32; 8], u32>,
}

impl Weld {
    fn push(&mut self, triangle: [MeshVertex; 3]) {
        for vertex in triangle {
            let key: [u32; 8] = bytemuck::cast(vertex);
            let at = *self.seen.entry(key).or_insert_with(|| {
                self.vertices.push(vertex);
                self.vertices.len() as u32 - 1
            });
            self.indices.push(at);
        }
    }
}
