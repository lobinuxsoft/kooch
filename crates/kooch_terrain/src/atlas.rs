//! The block atlas: one `R16Float` 3D texture whose tiles are the pool's slots, shared by every
//! planet, and an `R32Uint` twin of it holding each sample's material (#1951). A slot's tile never
//! moves, so a resident block is written once.

use glam::UVec3;

use super::TILE_SAMPLES;
use kooch_world::voxel::POOL_TEXTURE_FORMAT;

/// Tiles along X and Z; Y grows with the capacity.
const TILES_ACROSS: u32 = 32;

/// A sample's four strongest layers, packed: four ids of 5 bits, then three weights of 4 bits; the
/// fourth weight is what the others leave of 1.
pub const MATERIAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

pub struct BlockAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    material: wgpu::Texture,
    material_view: wgpu::TextureView,
    tiles: UVec3,
}

impl BlockAtlas {
    /// Room for `capacity` blocks, rounded up to whole layers of 32 × 32 tiles.
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let tiles = tiles_for(capacity);
        let size = tiles * TILE_SAMPLES;
        let create = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: size.z,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let texture = create("kooch_terrain::atlas", POOL_TEXTURE_FORMAT);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let material = create("kooch_terrain::atlas::material", MATERIAL_FORMAT);
        let material_view = material.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            material,
            material_view,
            tiles,
        }
    }

    /// Tiles per axis.
    pub fn tiles(&self) -> UVec3 {
        self.tiles
    }

    /// Blocks it holds.
    pub fn capacity(&self) -> u32 {
        self.tiles.x * self.tiles.y * self.tiles.z
    }

    /// First texel of a slot's tile; the generation shader decodes slots the same way.
    pub fn corner(&self, slot: u32) -> UVec3 {
        let tile = UVec3::new(
            slot % self.tiles.x,
            (slot / self.tiles.x) % self.tiles.y,
            slot / (self.tiles.x * self.tiles.y),
        );
        tile * TILE_SAMPLES
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn material(&self) -> &wgpu::Texture {
        &self.material
    }

    pub fn material_view(&self) -> &wgpu::TextureView {
        &self.material_view
    }
}

fn tiles_for(capacity: u32) -> UVec3 {
    let layer = TILES_ACROSS * TILES_ACROSS;
    UVec3::new(TILES_ACROSS, capacity.div_ceil(layer).max(1), TILES_ACROSS)
}
