//! The point lights' cube array: six depth faces per casting light
//! (#778).
//!
//! Separate from [`ShadowAtlas`](super::atlas::ShadowAtlas) rather than
//! more layers on it, and the reason is size. The cascades render at
//! 2048² because a cascade covers tens of metres of world; a cube face
//! covers a lamp's own room and renders at 512². One texture cannot have
//! two sizes, and sharing would mean six 2048² faces per light — 96 MiB
//! each, against 6 MiB at the size they actually need.
//!
//! # Why six culls and not twenty-four
//!
//! Each face culls from its own 90° frustum, so each needs a survivor
//! list, and a draw reading the list the next cull writes is what puts a
//! barrier between them. Six lets one light's faces overlap on the GPU,
//! which is where the parallelism is: the faces of a light are the six
//! draws that could run at once. Lights then serialise against each
//! other, which costs three barriers at `MAX_POINT_SHADOWS`, against
//! eighteen more survivor arenas standing idle in the common case where
//! no point light casts at all.

use crate::meshlet::MeshletCull;

use super::atlas::SHADOW_DEPTH_FORMAT;
use super::point::CUBE_FACES;

/// Side of one cube face, in texels.
///
/// 🔴 512 and not Bevy's 1024, and it is a memory decision rather than a
/// quality one. Six faces at `Depth32Float` is 6 MiB per light here and
/// 24 MiB at 1024 — four casting lights would add 96 MiB to a shadow
/// budget already at 128 MiB, on a handheld whose GPU memory is the
/// system's. At 512 a face is ~1.5 cm per texel five metres from the
/// lamp, which is finer than the contact shadows it sits beside.
pub const DEFAULT_CUBE_SIZE: u32 = 512;

/// The cube array, its per-face culls, and the views to render into.
pub struct PointShadowCubes {
    texture: wgpu::Texture,
    /// The sampling view: `CubeArray`, one binding.
    view: wgpu::TextureView,
    /// One per face of the light currently being drawn — see the module
    /// docs on why this is six and not `6 * MAX_POINT_SHADOWS`.
    culls: Vec<MeshletCull>,
    /// One `D2` view per layer, because a depth attachment is a single
    /// layer and the cube view above cannot be one.
    face_views: Vec<wgpu::TextureView>,
    size: u32,
    lights: u32,
}

impl PointShadowCubes {
    /// Allocates `lights` cubes and the six shared culls.
    pub fn new(
        device: &wgpu::Device,
        size: u32,
        lights: u32,
        initial_capacity: u32,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        let size = size.max(1);
        let lights = lights.max(1);
        let layers = lights * CUBE_FACES as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point_shadow_cubes"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                // A cube array is an ordinary 2D array whose layer count
                // is a multiple of six; the cube-ness is in the view.
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_DEPTH_FORMAT,
            // 🔴 `COPY_SRC` so a test can read the map itself.
            //
            // #853 was found by copying a face out and stretching its
            // contrast: every other picture available went through the
            // sampling path, the filter, the bias and a surface shader
            // first, and one of those — a grid normalised by the light's
            // `range` — turned a solid occluder into a crescent and sent
            // the search after a defect that was not there.
            //
            // ⚠️ The cost is not measured. Some drivers drop depth
            // compression on a texture that can be copied from; on the
            // handheld's 13.9 ms that is worth a number before it is
            // taken for granted.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("point_shadow_cubes_view"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let face_views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("point_shadow_face_view"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let culls = (0..CUBE_FACES)
            .map(|_| MeshletCull::new(device, initial_capacity.max(1), max_triangles_per_meshlet))
            .collect();

        Self {
            texture,
            view,
            culls,
            face_views,
            size,
            lights,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// Which array layer light `slot`'s `face` renders into.
    ///
    /// Faces of one light are contiguous, which is what
    /// `textureSampleCompareLevel` on a cube array requires: it takes
    /// the light index and picks the face itself from the direction.
    pub fn layer(slot: usize, face: usize) -> u32 {
        (slot * CUBE_FACES + face) as u32
    }

    /// The depth target for one face.
    pub fn face_view(&self, slot: usize, face: usize) -> &wgpu::TextureView {
        let index = (Self::layer(slot, face) as usize).min(self.face_views.len() - 1);
        &self.face_views[index]
    }

    /// The cull for one face. Shared across lights, so a light's six
    /// draws must be recorded before the next light's culls.
    pub fn cull(&self, face: usize) -> &MeshletCull {
        &self.culls[face.min(CUBE_FACES - 1)]
    }

    pub fn ensure_capacity(&mut self, device: &wgpu::Device, meshlets: u32, groups: u32) {
        for cull in &mut self.culls {
            cull.ensure_capacity(device, meshlets);
            cull.ensure_group_capacity(device, groups);
        }
    }

    /// Bytes the cube array occupies, for the VRAM tracker.
    pub fn byte_size(&self) -> u64 {
        let side = self.size as u64;
        side * side * 4 * CUBE_FACES as u64 * self.lights as u64
    }
}

#[cfg(test)]
mod tests;
