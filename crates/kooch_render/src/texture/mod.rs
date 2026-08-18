//! Texture loading + GPU upload.
//!
//! Wires the `image` crate (PNG / JPEG decoders) into the asset pipeline
//! introduced in #391: [`ImageLoader`] implements
//! [`AssetLoader<Image>`](kooch_core::asset_loader::AssetLoader) so game
//! code calls `AssetServer::load::<Image>(path)` and gets back a typed
//! [`Handle<Image>`](kooch_core::assets::Handle).
//!
//! The CPU [`Image`] holds raw RGBA8 pixels; [`GpuTexture::upload`] turns
//! it into a `wgpu::Texture` ready for sampling.
//!
//! # Scope (PR-1 of #131)
//!
//! Covered:
//! - PNG / JPEG decode via `image` crate
//! - sRGB and linear format hints (color vs data textures)
//! - GPU upload, with a mip chain when the import asks for one
//! - 1×1 solid-color factory for default textures
//!
//! Deferred (follow-ups):
//! - KTX2 + compressed formats (BC7, ASTC, ETC2)
//! - HDR (Rgba16Float / Rgba32Float) for IBL
//! - Default texture cache (1×1 white / normal / black) inserted at startup
//! - Sampler cache + bindless descriptor sets
//! - Async / streaming uploads

mod asset;
mod gpu_texture;
mod image_loader;
mod mipmap;

pub use asset::{Image, ImageFormat};
pub use gpu_texture::GpuTexture;
pub use image_loader::ImageLoader;
pub use mipmap::{Mipmapper, level_count};
