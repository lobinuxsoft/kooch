//! The baked cut as polygons in uv space (#452): marching squares over the mask, then simplified
//! under half a texel — further than that and the edge would move where the bake could not see it.
//!
//! `contour` walks the grid (a d3-contour port) and `geo` simplifies; neither is worth rewriting.

use contour::ContourBuilder;
use geo::{MultiPolygon, Simplify};

/// What the cut keeps, in uv, or `None` when nothing survives it.
pub(super) fn coverage(mask: &[u8], side: u32) -> Option<MultiPolygon<f64>> {
    let values: Vec<f64> = mask.iter().map(|&texel| f64::from(texel) / 255.0).collect();
    let step = 1.0 / f64::from(side);
    // A texel's value stands for its centre, so the grid starts half a texel inside the square.
    let builder = ContourBuilder::new(side as usize, side as usize, true)
        .x_origin(step * 0.5)
        .y_origin(step * 0.5)
        .x_step(step)
        .y_step(step);
    let bands = builder.contours(&values, &[0.5]).ok()?;
    let region = bands.first()?.geometry().clone();
    if region.0.is_empty() {
        return None;
    }
    Some(region.simplify(step * 0.5))
}

/// Whether every texel `uv` covers is kept, none of it is, or the cut runs through it. One texel of
/// margin: a bound that lands mid-texel still has the edge beside it.
pub(super) fn covers(mask: &[u8], side: u32, min: glam::Vec2, max: glam::Vec2) -> Cover {
    let texel = |at: f32| ((at * side as f32).floor() as i32).clamp(0, side as i32 - 1);
    let (x0, x1) = (texel(min.x) - 1, texel(max.x) + 1);
    let (y0, y1) = (texel(min.y) - 1, texel(max.y) + 1);
    let mut kept = 0u32;
    let mut cut = 0u32;
    for y in y0.max(0)..=y1.min(side as i32 - 1) {
        for x in x0.max(0)..=x1.min(side as i32 - 1) {
            match mask[(y as u32 * side + x as u32) as usize] >= 128 {
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

/// How the cut meets a triangle's uv.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cover {
    All,
    None,
    Edge,
}
