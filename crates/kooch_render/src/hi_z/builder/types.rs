use bytemuck::{Pod, Zeroable};

/// Push-constant-equivalent uniform consumed by the SPD shader. The
/// second `_padN` words round the type to 16 bytes — wgpu requires
/// uniform binding sizes to be 16-byte multiples.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub(super) struct SpdConstants {
    pub(super) max_mip_level: u32,
    pub(super) _pad0: u32,
    pub(super) _pad1: u32,
    pub(super) _pad2: u32,
}

pub struct HiZ {
    pub(super) texture: wgpu::Texture,
    pub(super) mip_views: Vec<wgpu::TextureView>,
    pub(super) full_view: wgpu::TextureView,

    /// Virtual source dimensions used by SPD's `load_mip_0` to round
    /// non-pow2 source sizes up. Equals `(source + 1).next_power_of_two()`
    /// per Bevy's reference. Drives the first-dispatch workgroup
    /// count: `virtual / 64`.
    pub(super) virtual_w: u32,
    pub(super) virtual_h: u32,

    // ── SPD path (production) ──────────────────────────────────────
    pub(super) spd_first_pipeline: wgpu::ComputePipeline,
    pub(super) spd_second_pipeline: wgpu::ComputePipeline,
    pub(super) spd_bgl: wgpu::BindGroupLayout,
    /// SPD's `max_mip_level` constants buffer. Written once at
    /// construction; never mutated per frame.
    pub(super) spd_constants_buffer: wgpu::Buffer,
    pub(super) spd_sampler: wgpu::Sampler,
    /// Dummy 1×1 R32Float texture for SPD slots beyond `mip_count`.
    /// Held here to keep the dummy view alive across frames.
    #[allow(dead_code)]
    pub(super) spd_dummy_texture: wgpu::Texture,
    pub(super) spd_dummy_view: wgpu::TextureView,

    // ── Legacy per-mip path (test-only) ────────────────────────────
    pub(super) copy_r32_pipeline: wgpu::ComputePipeline,
    pub(super) reduce_pipeline: wgpu::ComputePipeline,
    pub(super) copy_r32_bgl: wgpu::BindGroupLayout,
    #[allow(dead_code)] // kept alive so cached `reduce_bgs` stay valid
    pub(super) reduce_bgl: wgpu::BindGroupLayout,
    /// Pre-built bind groups for the legacy per-mip reduce path.
    /// Only used by `build_from_r32` (tests).
    pub(super) reduce_bgs: Vec<wgpu::BindGroup>,

    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) mip_count: u32,
}
