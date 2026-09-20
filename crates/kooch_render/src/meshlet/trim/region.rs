//! The baked coverage as a hull in uv space (#452): what the mesh has to keep, in as few corners as
//! a budget allows. Not the exact contour — the material still cuts and blends per pixel inside, and
//! a mesh that traced every texel would trade fill for vertices, which is the wrong way round.
//!
//! The hull **contains** the coverage, always. The mask is grown by a margin first and the contour
//! simplified by no more than that margin, so Douglas-Peucker cutting a corner can only eat back
//! into what the margin added. Too many corners for the budget: grow both and walk it again.
//!
//! `contour` walks the grid (a d3-contour port) and `geo` simplifies; neither is worth rewriting.

use contour::ContourBuilder;
use geo::{MultiPolygon, Simplify};

/// Texels the mask grows by, in the order the hull tries them. Each one is the simplification's own
/// budget, so a coarser hull is a wider one and never a tighter one.
const MARGINS: [u32; 4] = [2, 4, 8, 16];

/// The hull a mesh is cut against: the polygons and the grown mask the cut classifies triangles by.
pub(super) struct Hull {
    pub region: MultiPolygon<f64>,
    pub grown: Vec<u8>,
}

/// `mask`'s coverage as a hull of at most `budget` corners, or `None` when nothing is covered.
/// `threshold` is what counts as covered: the cut's own half for a masked material, a hair above
/// nothing for a transparent one, which contributes wherever its alpha is not zero.
pub(super) fn hull(mask: &[u8], side: u32, threshold: f64, budget: usize) -> Option<Hull> {
    let mut coarsest = None;
    for margin in MARGINS {
        let grown = dilate(mask, side, margin);
        let region = contour(&grown, side, threshold)?;
        let region = region.simplify(f64::from(margin) / f64::from(side));
        let corners: usize = region
            .0
            .iter()
            .map(|polygon| polygon.exterior().0.len())
            .sum();
        coarsest = Some(Hull { region, grown });
        if corners <= budget {
            break;
        }
    }
    coarsest
}

/// The mask grown by `margin` texels: a separable running maximum, so a hull built on it stands off
/// the coverage by at least that much.
fn dilate(mask: &[u8], side: u32, margin: u32) -> Vec<u8> {
    let at = |data: &[u8], x: u32, y: u32| data[(y * side + x) as usize];
    let reach = margin as i32;
    let mut wide = vec![0u8; mask.len()];
    for y in 0..side {
        for x in 0..side {
            let mut most = 0u8;
            for step in -reach..=reach {
                let read = (x as i32 + step).clamp(0, side as i32 - 1) as u32;
                most = most.max(at(mask, read, y));
            }
            wide[(y * side + x) as usize] = most;
        }
    }
    let mut grown = vec![0u8; mask.len()];
    for y in 0..side {
        for x in 0..side {
            let mut most = 0u8;
            for step in -reach..=reach {
                let read = (y as i32 + step).clamp(0, side as i32 - 1) as u32;
                most = most.max(at(&wide, x, read));
            }
            grown[(y * side + x) as usize] = most;
        }
    }
    grown
}

/// Marching squares at `threshold`, in uv. A texel's value stands for its centre, so the grid starts
/// half a texel inside the square.
fn contour(mask: &[u8], side: u32, threshold: f64) -> Option<MultiPolygon<f64>> {
    let values: Vec<f64> = mask.iter().map(|&texel| f64::from(texel) / 255.0).collect();
    let step = 1.0 / f64::from(side);
    let builder = ContourBuilder::new(side as usize, side as usize, true)
        .x_origin(step * 0.5)
        .y_origin(step * 0.5)
        .x_step(step)
        .y_step(step);
    let bands = builder.contours(&values, &[threshold]).ok()?;
    let region = bands.first()?.geometry().clone();
    (!region.0.is_empty()).then_some(region)
}

/// Whether every texel `uv` covers is kept, none of it is, or the hull's edge runs through it. One
/// texel of margin: a bound that lands mid-texel still has the edge beside it.
pub(super) fn covers(
    mask: &[u8],
    side: u32,
    threshold: f64,
    min: glam::Vec2,
    max: glam::Vec2,
) -> Cover {
    let texel = |at: f32| ((at * side as f32).floor() as i32).clamp(0, side as i32 - 1);
    let (x0, x1) = (texel(min.x) - 1, texel(max.x) + 1);
    let (y0, y1) = (texel(min.y) - 1, texel(max.y) + 1);
    let level = (threshold * 255.0) as u8;
    let mut kept = 0u32;
    let mut cut = 0u32;
    for y in y0.max(0)..=y1.min(side as i32 - 1) {
        for x in x0.max(0)..=x1.min(side as i32 - 1) {
            match mask[(y as u32 * side + x as u32) as usize] >= level {
                true => kept += 1,
                false => cut += 1,
            }
        }
    }
    match (kept, cut) {
        (0, _) => Cover::None,
        (_, 0) => Cover::All,
        _ => Cover::Edge,
    }
}

/// How the hull meets a triangle's uv.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cover {
    All,
    None,
    Edge,
}
