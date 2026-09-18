//! Render targets, pooled by descriptor (#392).
//!
//! 🔴 Why a pool at all: two views of the same size used to allocate two sets of targets, and a
//! resize allocated a new set and kept the old one alive for three frames. Reusing a slot means
//! N views of one size cost one set, and a resize settles back to one.
//!
//! A pool also gives a pass somewhere to draw: a post-process asks for a target by descriptor
//! rather than creating a texture it then has to own.

mod slots;

use slots::{Fresh, Slots};

pub use kooch_plugin_render::Targets;
pub use slots::{RETIREMENT, TargetDesc, TargetId};

/// Textures and views, reused by descriptor.
pub struct TargetPool {
    /// Kept so a pass asks for a target with a label and a descriptor and nothing else. A device is
    /// an `Arc` handle, so this is a refcount.
    device: wgpu::Device,
    slots: Slots,
    /// Parallel to the slots: SoA rather than a struct per target, and a slot's texture outlives
    /// every release so the retirement has something to keep.
    textures: Vec<wgpu::Texture>,
    views: Vec<wgpu::TextureView>,
    created: u32,
}

impl TargetPool {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            slots: Slots::default(),
            textures: Vec::new(),
            views: Vec::new(),
            created: 0,
        }
    }

    /// A target matching `desc`, reused when the pool has a free one and created otherwise.
    pub fn acquire(&mut self, label: &str, desc: TargetDesc) -> TargetId {
        let (index, fresh) = self.slots.claim(desc);
        if fresh == Fresh::Created {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: desc.size.0.max(1),
                    height: desc.size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: desc.mips,
                sample_count: desc.samples,
                dimension: wgpu::TextureDimension::D2,
                format: desc.format,
                usage: desc.usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.textures.push(texture);
            self.views.push(view);
            self.created += 1;
        }
        TargetId(index)
    }

    /// Hands a target back. It waits out [`RETIREMENT`] frames before anything reuses it, which is
    /// what Mesa radv needs from a texture a bind group may still name.
    pub fn release(&mut self, target: TargetId) {
        self.slots.release(target.0);
    }

    pub fn view(&self, target: TargetId) -> Option<&wgpu::TextureView> {
        self.views.get(target.0 as usize)
    }

    pub fn texture(&self, target: TargetId) -> Option<&wgpu::Texture> {
        self.textures.get(target.0 as usize)
    }

    pub fn desc(&self, target: TargetId) -> Option<TargetDesc> {
        self.slots.desc(target.0)
    }

    /// Rotates the retirement ring. Called once per frame, after the last submit.
    pub fn end_frame(&mut self) {
        self.slots.end_frame();
    }

    /// How many textures the pool holds — what the VRAM it owns is counted from.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// How many textures were ever created. A frame loop that keeps climbing here is allocating
    /// every frame, which is the failure this pool exists to make visible.
    pub fn created(&self) -> u32 {
        self.created
    }

    /// Bytes the pool's textures occupy, by descriptor.
    pub fn bytes(&self) -> u64 {
        (0..self.slots.len())
            .filter_map(|index| self.slots.desc(index as u32))
            .map(|desc| {
                // `block_copy_size`, not `target_pixel_byte_cost`: the latter is wgpu's render
                // target budget, which charges 8 bytes for a 4-byte format.
                let block = desc.format.block_copy_size(None).unwrap_or(4) as u64;
                block * desc.size.0.max(1) as u64 * desc.size.1.max(1) as u64
            })
            .sum()
    }
}

/// 🔴 The same pool a plugin's pass draws into. The trait is the plugin-facing half, so a pass sees
/// handles and never the `Vec`s behind them.
impl Targets for TargetPool {
    fn acquire(&mut self, label: &str, desc: TargetDesc) -> TargetId {
        TargetPool::acquire(self, label, desc)
    }

    fn view(&self, target: TargetId) -> Option<&wgpu::TextureView> {
        TargetPool::view(self, target)
    }

    fn release(&mut self, target: TargetId) {
        TargetPool::release(self, target)
    }
}
