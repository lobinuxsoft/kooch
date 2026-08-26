//! The froxel grid's dimensions and the factors that map a fragment to
//! one of its cells.
//!
//! No GPU here on purpose: everything below is arithmetic over the
//! camera and the viewport, which is what makes the z-slice mapping
//! testable at all. The shader reproduces `z_slice` exactly, and the
//! tests next door are what pin it.

use glam::{UVec3, Vec2};

/// How the grid is sized, as a
/// [`Resource`](kooch_core::resource::Resources).
///
/// Absent, the default is used: 24 slices deep and about 4096 cells,
/// which is Bevy's and is the shape the technique was measured with.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClusterSettings {
    /// Cells across the whole grid, before the aspect ratio splits them
    /// into columns and rows. A budget, not a dimension.
    pub total: u32,
    /// Cells along the view axis.
    pub z_slices: u32,
    /// How far the first slice reaches, in metres.
    ///
    /// The grid's near plane, and not the camera's: a camera near plane
    /// of a centimetre would spend most of the slices on the first metre
    /// of the frustum, where a logarithmic distribution puts them.
    pub first_slice: f32,
    /// How far the grid reaches, in metres.
    ///
    /// 🔴 Kóoch projects with an infinite reversed-Z frustum (ADR 0002),
    /// so there is no far plane to read this off. Bevy reads back the
    /// furthest light the GPU saw and resizes the grid next frame; that
    /// is a readback in the hot path, which this renderer does not do.
    ///
    /// What it costs: a light further out than this lands in the last
    /// slice along with everything else behind it, so that slice's cells
    /// hold more lights than they should. Nothing renders wrong — the
    /// cell is conservative, never exclusive — it just stops saving
    /// work out there.
    pub far: f32,
    /// Whether to build the grid at all.
    ///
    /// Off falls back to the linear walk over every light — the same
    /// image, at the cost clustering exists to remove. It is here to be
    /// the A/B: a capture with it on and one with it off, same camera,
    /// same scene, is the only honest way to say what the grid bought.
    /// `KOOCH_CLUSTERING=off` sets it from outside a build.
    pub enabled: bool,
}

impl Default for ClusterSettings {
    fn default() -> Self {
        Self {
            total: 4096,
            z_slices: 24,
            first_slice: 5.0,
            far: 200.0,
            enabled: !disabled_by_environment(),
        }
    }
}

/// `KOOCH_CLUSTERING=off` (or `0`), read once.
///
/// An environment variable rather than only a setting because the
/// comparison it exists for is made on a handheld, over SSH, against a
/// build nobody wants to make twice.
fn disabled_by_environment() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        matches!(
            std::env::var("KOOCH_CLUSTERING").as_deref(),
            Ok("off") | Ok("0") | Ok("false")
        )
    })
}

/// The grid a single view is clustered with this frame.
///
/// Derived from the settings and the viewport, so two viewports of
/// different sizes get different grids from the same settings — which is
/// the point: the cells are meant to be roughly square on screen.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClusterGrid {
    pub dimensions: UVec3,
    /// `dimensions.xy / viewport`, so a fragment's tile is a multiply.
    pub tile_factors: Vec2,
    /// The two constants of the logarithmic z-slice mapping.
    pub z_factors: Vec2,
    pub near: f32,
    pub far: f32,
}

impl ClusterGrid {
    /// Sizes the grid for a viewport in pixels.
    pub fn new(settings: &ClusterSettings, viewport: Vec2) -> Self {
        let dimensions = dimensions_for(settings, viewport);
        let near = settings.first_slice.max(0.01);
        let far = settings.far.max(near * 2.0);
        Self {
            dimensions,
            tile_factors: dimensions.truncate().as_vec2() / viewport.max(Vec2::ONE),
            z_factors: z_factors(near, far, dimensions.z),
            near,
            far,
        }
    }

    /// Cells in the whole grid — the length every per-cluster buffer is
    /// sized to.
    pub fn cluster_count(&self) -> u32 {
        self.dimensions.x * self.dimensions.y * self.dimensions.z
    }

    /// The slice a view-space depth falls in.
    ///
    /// `view_z` is negative in front of the camera, which is why the
    /// logarithm takes its negation. Mirrors `cluster_z_slice` in
    /// `cluster_common.wgsl`; the two must agree or a fragment reads a
    /// cell the grid never wrote.
    pub fn z_slice(&self, view_z: f32) -> u32 {
        let slice = (-view_z).ln() * self.z_factors.x - self.z_factors.y + 1.0;
        // A negative `slice` is a fragment nearer than the first slice's
        // depth, and `as u32` on a negative float saturates to 0 in Rust
        // — which is the answer we want, but the shader's `u32()` does
        // not promise it. `max` first, in both.
        (slice.max(0.0) as u32).min(self.dimensions.z - 1)
    }

    /// How deep a froxel is, in metres, at `distance` from the camera.
    ///
    /// The number that makes `far` mean something. "200 versus 40" says
    /// nothing on its own; "2.6 metres of froxel against a 4 metre
    /// light" says the whole problem — a light is charged to every pixel
    /// of every cell it reaches, so a slice thicker than the light it
    /// holds spreads that light over depth it never lit (#820).
    ///
    /// Inverts the slice mapping: `z = exp((slice + y - 1) / x)`, and the
    /// thickness is the gap between this slice's near and far edges.
    pub fn slice_depth(&self, distance: f32) -> f32 {
        if self.z_factors.x <= 0.0 {
            return self.far - self.near;
        }
        let distance = distance.abs();
        // 🔴 The first and last slices are not what the formula says.
        // Slice 0 holds *everything* nearer than `near` and the last one
        // everything past `far` — the mapping describes the slices in
        // between, and reporting its answer for the two ends understates
        // them by however much geometry is piled there.
        //
        // This mattered immediately: with the grid starting at 20 m over
        // a scene 10 m away, the panel reported a 0.9 m froxel while
        // every pixel of that scene was in one 20 m cell. The reading
        // that would have explained the screen was the one the tool
        // refused to give.
        if distance < self.near {
            return self.near;
        }
        let slice = self.z_slice(-distance) as f32;
        if slice >= (self.dimensions.z - 1) as f32 {
            return f32::INFINITY;
        }
        let edge = |s: f32| ((s + self.z_factors.y - 1.0) / self.z_factors.x).exp();
        edge(slice + 1.0) - edge(slice)
    }
}

/// Splits `total` cells into columns and rows that stay roughly square
/// on screen, with `z_slices` along the view axis.
fn dimensions_for(settings: &ClusterSettings, viewport: Vec2) -> UVec3 {
    let z = settings.z_slices.clamp(1, settings.total.max(1));
    let per_layer = (settings.total.max(1) as f32 / z as f32).max(1.0);
    let aspect = (viewport.x / viewport.y.max(1.0)).max(0.01);

    let rows = (per_layer / aspect).sqrt();
    let mut x = (rows * aspect) as u32;
    let mut y = rows as u32;
    // A viewport thin enough in either axis rounds one of them to zero,
    // which would make `cluster_count` zero and every per-cluster buffer
    // empty. One row of many columns is a degenerate grid; an empty one
    // is a division by zero in the allocation pass.
    if x == 0 {
        x = 1;
        y = per_layer as u32;
    }
    if y == 0 {
        x = per_layer as u32;
        y = 1;
    }
    UVec3::new(x.max(1), y.max(1), z)
}

/// The two constants of the logarithmic slice mapping.
///
/// Slices are distributed the way depth precision falls off rather than
/// by metres, so a cell near the camera is thin and one far away is
/// thick. `slice = ln(-z) * x - y + 1`, and the `+ 1` is what leaves
/// slice 0 for everything nearer than `near`.
fn z_factors(near: f32, far: f32, z_slices: u32) -> Vec2 {
    let scale = (z_slices as f32 - 1.0) / (far / near).ln();
    Vec2::new(scale, near.ln() * scale)
}

#[cfg(test)]
mod tests;
