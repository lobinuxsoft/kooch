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
pub use slots::{TargetDesc, TargetId};

/// Textures and views, reused by descriptor.
pub struct TargetPool {
    /// Kept so a pass asks for a target with a label and a descriptor and nothing else. A device is
    /// an `Arc` handle, so this is a refcount.
    device: wgpu::Device,
    slots: Slots,
    /// Parallel to the slots: SoA rather than a struct per target. A slot's texture outlives every
    /// release — the pool never destroys one, which is why reuse needs no wait.
    textures: Vec<Option<wgpu::Texture>>,
    views: Vec<Option<wgpu::TextureView>>,
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
            // An evicted slot's index is reused, so a new texture lands where the old one was.
            match self.textures.get_mut(index as usize) {
                Some(slot) => {
                    *slot = Some(texture);
                    self.views[index as usize] = Some(view);
                }
                None => {
                    self.textures.push(Some(texture));
                    self.views.push(Some(view));
                }
            }
            self.created += 1;
            // 🔴 A pool that keeps growing is a pass acquiring without releasing, and that ran a
            // machine out of memory before anything said so (#1201). Counted live, not created: a
            // resize creates and evicts, and is not a leak. Loud at every doubling from 32 on.
            let live = self.len();
            if live >= 32 && live.is_power_of_two() {
                tracing::error!(
                    live,
                    label,
                    size = ?desc.size,
                    "the target pool keeps growing: a pass is acquiring without releasing"
                );
            }
        }
        TargetId(index)
    }

    /// Hands a target back. The next request for the same descriptor gets it.
    pub fn release(&mut self, target: TargetId) {
        self.slots.release(target.0);
    }

    pub fn view(&self, target: TargetId) -> Option<&wgpu::TextureView> {
        self.views.get(target.0 as usize)?.as_ref()
    }

    pub fn texture(&self, target: TargetId) -> Option<&wgpu::Texture> {
        self.textures.get(target.0 as usize)?.as_ref()
    }

    /// Closes a frame: drops the textures no pass asked for in the last few. Called once a frame by
    /// whoever presents it.
    pub fn end_frame(&mut self) {
        for index in self.slots.end_frame() {
            self.textures[index as usize] = None;
            self.views[index as usize] = None;
        }
    }

    pub fn desc(&self, target: TargetId) -> Option<TargetDesc> {
        self.slots.desc(target.0)
    }

    /// How many textures the pool holds — what the VRAM it owns is counted from.
    pub fn len(&self) -> usize {
        self.textures.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
