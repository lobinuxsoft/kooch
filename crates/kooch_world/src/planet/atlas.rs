//! The block atlas: one `R16Float` 3D texture whose tiles are the pool's slots, shared by every
//! planet. A slot's tile never moves, so a resident block is written once.

use glam::UVec3;

use super::TILE_SAMPLES;
use crate::voxel::POOL_TEXTURE_FORMAT;

/// Tiles along X and Z; Y grows with the capacity.
const TILES_ACROSS: u32 = 32;

pub struct BlockAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    tiles: UVec3,
}

impl BlockAtlas {
    /// Room for `capacity` blocks, rounded up to whole layers of 32 × 32 tiles.
    pub fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let tiles = tiles_for(capacity);
        let size = tiles * TILE_SAMPLES;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kooch_world::planet::atlas"),
            size: wgpu::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: size.z,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: POOL_TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
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
}

fn tiles_for(capacity: u32) -> UVec3 {
    let layer = TILES_ACROSS * TILES_ACROSS;
    UVec3::new(TILES_ACROSS, capacity.div_ceil(layer).max(1), TILES_ACROSS)
}
