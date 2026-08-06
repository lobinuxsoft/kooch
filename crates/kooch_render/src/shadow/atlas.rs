//! The shadow depth atlas: one texture, one cascade per quadrant.
//!
//! # Why an atlas and not four textures
//!
//! The shading pass samples whichever cascade covers the fragment, and
//! which one that is varies per pixel. Four separate textures would mean
//! four bindings and a dynamic index into them, which WGSL only offers
//! through binding arrays. One texture with four viewports is a single
//! binding and a scale-and-bias on the uv — and, on the bind-group
//! budget this engine has left, one binding is the difference between
//! fitting and not.
//!
//! # Why four culls
//!
//! Each cascade culls from its own light-space frustum, so each needs
//! its own survivor list. They could share one `MeshletCull` used four
//! times, and that would serialise the whole pass: cascade 0's indirect
//! draw reads the same `visible_meshlets` that cascade 1's cull writes,
//! so wgpu inserts a barrier between every pair. Four culls are buffers
//! — no textures — and let the cascades overlap on the GPU.

use crate::meshlet::MeshletCull;

use super::cascades::CASCADE_COUNT;

/// Depth format for the atlas.
///
/// `Depth32Float` rather than the 16-bit variant: the comparison happens
/// against a reconstructed world position, and 16 bits of depth over a
/// cascade that can span hundreds of metres quantises into visible
/// stair-stepping on the shadow of anything at a shallow angle.
pub const SHADOW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Side of one cascade, in texels. The atlas is twice this on each axis.
///
/// 2048 per cascade means a 4096² atlas — 64 MiB at `Depth32Float`. That
/// is a real cost and it is the default because the alternative is
/// visible: at 1024 the near cascade is already soft enough that contact
/// shadows look detached.
pub const DEFAULT_CASCADE_SIZE: u32 = 2048;

/// Where a cascade lives in the atlas, in texels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasRegion {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

impl AtlasRegion {
    /// Scale and bias that maps a cascade's `[0,1]` shadow uv into the
    /// atlas's `[0,1]`. The shader multiplies then adds; packing it this
    /// way keeps the sample site to one `fma` rather than a branch on
    /// which quadrant.
    pub fn uv_scale_bias(&self, atlas_size: u32) -> [f32; 4] {
        let atlas = atlas_size.max(1) as f32;
        let scale = self.size as f32 / atlas;
        [scale, scale, self.x as f32 / atlas, self.y as f32 / atlas]
    }
}

/// The atlas texture, its per-cascade culls, and where each cascade sits.
pub struct ShadowAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// One per cascade. See the module docs for why these are not one.
    culls: Vec<MeshletCull>,
    cascade_size: u32,
    regions: [AtlasRegion; CASCADE_COUNT],
}

impl ShadowAtlas {
    /// Allocates the atlas and one cull per cascade.
    ///
    /// `initial_capacity` sizes the survivor lists; they grow later like
    /// every other buffer here, so a low guess costs a reallocation
    /// rather than a panic. The cull pipelines are shared and live on
    /// the render stage — nine compute pipelines per cascade is exactly
    /// what `MeshletCullPipelines` exists to avoid.
    pub fn new(
        device: &wgpu::Device,
        cascade_size: u32,
        initial_capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        let cascade_size = cascade_size.max(1);
        let atlas_size = cascade_size * 2;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_atlas"),
            size: wgpu::Extent3d {
                width: atlas_size,
                height: atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let culls = (0..CASCADE_COUNT)
            .map(|_| MeshletCull::new(device, initial_capacity.max(1), max_triangles_per_meshlet))
            .collect();

        Self {
            texture,
            view,
            culls,
            cascade_size,
            regions: quadrants(cascade_size),
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn cascade_size(&self) -> u32 {
        self.cascade_size
    }

    pub fn atlas_size(&self) -> u32 {
        self.cascade_size * 2
    }

    pub fn region(&self, cascade: usize) -> AtlasRegion {
        self.regions[cascade.min(CASCADE_COUNT - 1)]
    }

    pub fn cull(&self, cascade: usize) -> &MeshletCull {
        &self.culls[cascade.min(CASCADE_COUNT - 1)]
    }

    pub fn cull_mut(&mut self, cascade: usize) -> &mut MeshletCull {
        let index = cascade.min(CASCADE_COUNT - 1);
        &mut self.culls[index]
    }

    /// Bytes the atlas occupies, for the VRAM tracker.
    pub fn byte_size(&self) -> u64 {
        let side = self.atlas_size() as u64;
        side * side * 4
    }
}

/// Quadrant layout: cascade 0 top-left, 1 top-right, 2 bottom-left,
/// 3 bottom-right.
///
/// Reading order, so a capture of the atlas is inspectable by eye — the
/// near cascade is the small tight one at the top left, and if it is not
/// there, the split scheme is upside down.
fn quadrants(cascade_size: u32) -> [AtlasRegion; CASCADE_COUNT] {
    let mut regions = [AtlasRegion {
        x: 0,
        y: 0,
        size: cascade_size,
    }; CASCADE_COUNT];
    for (i, region) in regions.iter_mut().enumerate() {
        region.x = (i as u32 % 2) * cascade_size;
        region.y = (i as u32 / 2) * cascade_size;
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrants_tile_the_atlas_without_overlapping() {
        let regions = quadrants(2048);
        for (i, a) in regions.iter().enumerate() {
            for b in regions.iter().skip(i + 1) {
                let disjoint = a.x + a.size <= b.x
                    || b.x + b.size <= a.x
                    || a.y + a.size <= b.y
                    || b.y + b.size <= a.y;
                assert!(disjoint, "cascades overlap: {a:?} and {b:?}");
            }
        }
        // And they cover it: four quadrants of a 2× square.
        let covered: u32 = regions.iter().map(|r| r.size * r.size / 1024).sum();
        assert_eq!(covered, (4096 * 4096) / 1024);
    }

    #[test]
    fn the_near_cascade_is_top_left() {
        let regions = quadrants(2048);
        assert_eq!(
            regions[0],
            AtlasRegion {
                x: 0,
                y: 0,
                size: 2048
            }
        );
    }

    /// The uv transform has to land a cascade's full `[0,1]` inside its
    /// own quadrant and nowhere else. Getting the bias wrong samples a
    /// neighbouring cascade, which looks like a shadow from the wrong
    /// distance rather than like a broken transform.
    #[test]
    fn uv_transform_maps_each_cascade_into_its_own_quadrant() {
        let regions = quadrants(2048);
        let atlas = 4096;
        for (i, region) in regions.iter().enumerate() {
            let [sx, sy, bx, by] = region.uv_scale_bias(atlas);
            let corner = |u: f32, v: f32| (u * sx + bx, v * sy + by);

            let (x0, y0) = corner(0.0, 0.0);
            let (x1, y1) = corner(1.0, 1.0);
            let expect_x0 = region.x as f32 / atlas as f32;
            let expect_y0 = region.y as f32 / atlas as f32;

            assert!((x0 - expect_x0).abs() < 1e-6, "cascade {i} u origin");
            assert!((y0 - expect_y0).abs() < 1e-6, "cascade {i} v origin");
            assert!(
                (x1 - (expect_x0 + 0.5)).abs() < 1e-6,
                "cascade {i} must span exactly half the atlas, got {x1}",
            );
            assert!(
                (y1 - (expect_y0 + 0.5)).abs() < 1e-6,
                "cascade {i} v extent"
            );
            assert!((0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&y1));
        }
    }

    #[test]
    fn the_atlas_is_twice_a_cascade_on_each_axis() {
        let regions = quadrants(1024);
        let max_x = regions.iter().map(|r| r.x + r.size).max().unwrap();
        let max_y = regions.iter().map(|r| r.y + r.size).max().unwrap();
        assert_eq!((max_x, max_y), (2048, 2048));
    }
}
