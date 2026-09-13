//! Grid dimensions and the fragment-to-cell factors, GPU-free so the z-slice mapping is testable;
//! the shader reproduces `z_slice` exactly.

use glam::{UVec3, Vec2};

/// Grid sizing as a [`Resource`](kooch_core::resource::Resources); absent, 24 slices and ~4096
/// cells, Bevy's measured shape.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClusterSettings {
    /// Cells across the whole grid, before the aspect ratio splits them
    /// into columns and rows. A budget, not a dimension.
    pub total: u32,
    /// Cells along the view axis.
    pub z_slices: u32,
    /// First slice depth in metres, not the camera near plane, or log slices crowd the first metre.
    pub first_slice: f32,
    /// Grid reach in metres — the reverse-Z frustum has no far plane. Bevy resizes from the
    /// furthest light each frame; here it is a setting. Lights beyond land in the last slice:
    /// correct, just unsaved work.
    pub far: f32,
    /// Whether to build the grid; off is the linear walk, the same image — the A/B for what the
    /// grid bought. `KOOCH_CLUSTERING=off` sets it.
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

/// `KOOCH_CLUSTERING=off` (or `0`), read once — the comparison is made on a handheld over SSH, one
/// build.
fn disabled_by_environment() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        matches!(
            std::env::var("KOOCH_CLUSTERING").as_deref(),
            Ok("off") | Ok("0") | Ok("false")
        )
    })
}

/// The grid one view uses, from settings and viewport, so cells stay roughly square on screen.
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

    /// Slice for a view-space depth (negative ahead). Mirrors `cluster_z_slice`; they must agree.
    pub fn z_slice(&self, view_z: f32) -> u32 {
        let slice = (-view_z).ln() * self.z_factors.x - self.z_factors.y + 1.0;
        // `max` first: Rust saturates negative `as u32` to 0, WGSL's `u32()` does not promise to.
        (slice.max(0.0) as u32).min(self.dimensions.z - 1)
    }

    /// Froxel depth in metres at `distance` (#820) — a slice thicker than its light spreads that
    /// light over depth it never lit. `z = exp((slice + y - 1) / x)`.
    pub fn slice_depth(&self, distance: f32) -> f32 {
        if self.z_factors.x <= 0.0 {
            return self.far - self.near;
        }
        let distance = distance.abs();
        // 🔴 Slice 0 holds everything nearer than `near`, the last everything past `far`; the
        // formula understates both — the panel said 0.9 m while the scene sat in one 20 m cell.
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
    // A thin viewport rounds an axis to zero, emptying every buffer and dividing by zero in
    // allocation.
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

/// Log slice constants: thin near, thick far. `slice = ln(-z) * x - y + 1`, the `+ 1` leaving slice
/// 0 for depths nearer than `near`.
fn z_factors(near: f32, far: f32, z_slices: u32) -> Vec2 {
    let scale = (z_slices as f32 - 1.0) / (far / near).ln();
    Vec2::new(scale, near.ln() * scale)
}

#[cfg(test)]
mod tests;
