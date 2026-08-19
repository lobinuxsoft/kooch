//! Every intermediate FSR 3.1 keeps between its passes.
//!
//! Split out of `fsr3.rs` because fifteen textures with three different
//! grids is a thing worth reading on its own, and because the memory it
//! costs is a number someone will want to find.
//!
//! # 🔴 Why so much of this is `Rgba16Float` for one channel of data
//!
//! FSR stores these as `R16_FLOAT` and `R8_UNORM`. Neither is a storage
//! format in WebGPU without `TEXTURE_FORMATS_TIER1`, and the ones that
//! ARE storage formats below 32 bits are all four-channel. Meanwhile
//! `R32Float` is a storage format but is **not filterable** without
//! `FLOAT32_FILTERABLE`, and four of these are read through a linear
//! sampler.
//!
//! Storage ∩ filterable ∩ under 32 bits per channel leaves exactly
//! `Rgba16Float`. So anything sampled pays for four channels to use
//! one, rather than the engine growing a required feature for it.
//! `dilated_depth` and `reconstructed_depth` escape it by never being
//! sampled — they are `textureLoad`ed, where filtering does not apply.
//!
//! # What this costs, at 858×480 → 1280×720
//!
//! | Grid | Bytes |
//! |---|---|
//! | render resolution (nine targets) | ~30 MB |
//! | half render (one) | ~1.6 MB |
//! | output resolution (three) | ~18 MB |
//!
//! ⚠️ **~50 MB against SGSR 2's ~7.** That is the price of FSR's extra
//! machinery — locks, four-deep luma history, reactivity, an
//! accumulation counter.
//!
//! 🔴 **And it is paid whether or not the technique is selected**, the
//! same way SGSR 2's pair is: `Vbuf64Stage` owns both as plain fields
//! and allocates them on resize. Making it conditional means
//! `set_upscale` taking a device, which is a signature change across
//! two crates and every test that drives it — worth doing when there is
//! a memory number that says so, not on the way past.

use wgpu::TextureUsages;

use crate::meshlet::deferred::HDR_COLOR_FORMAT;

const STORAGE_AND_SAMPLED: TextureUsages =
    TextureUsages::STORAGE_BINDING.union(TextureUsages::TEXTURE_BINDING);

/// A texture plus the view every pass binds it through. Paired because
/// nothing here ever wants one without the other.
pub(super) struct Target {
    pub(super) texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
}

impl Target {
    fn new(
        device: &wgpu::Device,
        label: &str,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: STORAGE_AND_SAMPLED,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

/// A resource with a previous and a current half. The index flips once
/// per frame; nothing else may write it.
pub(super) struct Pair {
    targets: [Target; 2],
}

impl Pair {
    fn new(
        device: &wgpu::Device,
        label: &str,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            targets: [
                Target::new(device, &format!("{label}_0"), size, format),
                Target::new(device, &format!("{label}_1"), size, format),
            ],
        }
    }

    pub(super) fn view(&self, index: usize) -> &wgpu::TextureView {
        &self.targets[index].view
    }

    pub(super) fn texture(&self, index: usize) -> &wgpu::Texture {
        &self.targets[index].texture
    }
}

pub(super) struct Targets {
    /// `.xy` dilated motion, `.z` farthest depth in metres.
    pub(super) dilated: Target,
    /// The nearest device depth of the 3×3, reversed-Z.
    pub(super) dilated_depth: Target,
    /// This frame's depth scattered into the previous frame's grid,
    /// as raw float bits so `textureAtomicMax` can order it.
    pub(super) reconstructed_depth: Target,
    pub(super) current_luma: Target,
    /// `reactive`, `disocclusion`, `shading change`, `accumulation`.
    pub(super) reactive_masks: Target,
    pub(super) luma_instability: Target,
    /// Half render resolution.
    pub(super) farthest_mip1: Target,
    pub(super) accumulation: Pair,
    /// Four frames of luma per pixel, oldest in `.w`.
    pub(super) luma_history: Pair,
    /// Output resolution. Written by a render-resolution pass, so it
    /// has to be cleared rather than overwritten.
    pub(super) new_locks: Target,
    /// Output resolution. `rgb` is the resolved image — what the
    /// tonemap reads — and `a` carries the lock into the next frame.
    pub(super) history: Pair,
}

impl Targets {
    pub(super) fn new(device: &wgpu::Device, render: (u32, u32), output: (u32, u32)) -> Self {
        let half = (render.0.max(2) / 2, render.1.max(2) / 2);
        Self {
            dilated: Target::new(device, "fsr3_dilated", render, HDR_COLOR_FORMAT),
            dilated_depth: Target::new(
                device,
                "fsr3_dilated_depth",
                render,
                wgpu::TextureFormat::R32Float,
            ),
            reconstructed_depth: Target::new(
                device,
                "fsr3_reconstructed_depth",
                render,
                wgpu::TextureFormat::R32Uint,
            ),
            current_luma: Target::new(device, "fsr3_current_luma", render, HDR_COLOR_FORMAT),
            reactive_masks: Target::new(device, "fsr3_reactive_masks", render, HDR_COLOR_FORMAT),
            luma_instability: Target::new(
                device,
                "fsr3_luma_instability",
                render,
                HDR_COLOR_FORMAT,
            ),
            farthest_mip1: Target::new(device, "fsr3_farthest_mip1", half, HDR_COLOR_FORMAT),
            accumulation: Pair::new(device, "fsr3_accumulation", render, HDR_COLOR_FORMAT),
            luma_history: Pair::new(device, "fsr3_luma_history", render, HDR_COLOR_FORMAT),
            new_locks: Target::new(
                device,
                "fsr3_new_locks",
                output,
                wgpu::TextureFormat::R32Float,
            ),
            history: Pair::new(device, "fsr3_history", output, HDR_COLOR_FORMAT),
        }
    }
}
