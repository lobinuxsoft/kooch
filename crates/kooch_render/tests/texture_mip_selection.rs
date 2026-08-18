//! Which mip level the shading path asks for (#481 follow-up).
//!
//! Reported from the editor: a textured floor looks identical however
//! close the camera gets, and turning the chain off brings the detail
//! back. Three faults produce that picture — no chain, a wrong chain, or
//! a chain that is fine while the selection asks for the wrong level —
//! and `texture_mipmaps` already rules out the first two by reading the
//! levels back. This file is about the third.
//!
//! It renders the `Texture mip level` debug view, which paints the level
//! the sampler is being asked for, and compares two camera distances.
//! The claim is not "the level is right"; it is the weaker and more
//! useful one: **the level responds to the camera at all**.
//!
//! Run with:
//!   cargo test -p kooch_render --test texture_mip_selection

mod common;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

use glam::Vec3;
use kooch_core::Guid;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletDebugMode, ShadingRate};
use kooch_render::texture::{Image, ImageFormat};

/// A 256x256 one-pixel checker: the finest detail a chain can lose, so
/// the level being sampled decides what comes back.
fn checker(side: u32) -> Image {
    let mut px = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let v = if (x + y) % 2 == 0 { 0u8 } else { 255 };
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Image::from_rgba8(px, side, side, ImageFormat::Rgba8UnormSrgb)
}

/// The mean red of the frame. In the mip view red climbs with the level,
/// so this is a proxy for "which level is the frame sampling".
fn mean_red(image: &[u8]) -> f64 {
    let sum: u64 = image.chunks_exact(4).map(|p| u64::from(p[0])).sum();
    sum as f64 / (image.len() / 4) as f64
}

/// Renders the mip view with the camera at `eye`.
fn mip_view_at(eye: Vec3) -> Option<Vec<u8>> {
    let mut r = common::lit_scene::rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);

    // A textured floor, put straight into the pool: the material sync
    // resolves GUIDs through the asset server and off the disk, which is
    // a filesystem this test has no business needing.
    let texture = Guid::new_v4();
    let material = Guid::new_v4();
    {
        let pipeline = r
            .resources
            .get_mut::<MaterialPipeline>()
            .expect("the rig registers a material pipeline");
        pipeline.register_texture(&r.device, &r.queue, texture, &checker(256));
        pipeline.register(
            &r.queue,
            material,
            &Material::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.9, 0.0).with_albedo(texture),
        );
    }
    r.resources.insert(MeshletDebugMode::TextureMipLevel);
    r.camera = ViewCamera::looking_at(eye, Vec3::new(0.0, 0.5, 0.0));

    // Settled first: the opening frames upload the meshlets and the
    // textures, and read back black.
    let mut last = Vec::new();
    for _ in 0..3 {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        last = common::read_rgba8(&r.device, &r.queue, r.stage.color_texture());
    }
    Some(last)
}

/// 🔴 The level the sampler is asked for has to move with the camera.
///
/// This is the whole question, reduced to something a machine can
/// answer. A correct frame samples low levels up close and high ones at
/// distance, so the two renders differ; a selection that is saturated —
/// or frozen, or computed from something that is not the camera — paints
/// the same colour from any distance, which is exactly the picture that
/// was reported.
#[test]
#[ignore = "reproduces an open bug: the selection is inverted, near 255 / far 16.5"]
fn the_mip_level_responds_to_the_camera() {
    let _gpu = gpu_lock();
    let Some(near) = mip_view_at(Vec3::new(0.0, 1.0, 2.0)) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(far) = mip_view_at(Vec3::new(0.0, 12.0, 40.0)) else {
        return;
    };

    let (n, f) = (mean_red(&near), mean_red(&far));
    eprintln!("mip view — near {n:.2}, far {f:.2} (red climbs with the level)");
    assert!(
        (n - f).abs() > 1.0,
        "the mip view paints the same thing from 2 m and from 40 m ({n:.2} vs {f:.2}), so \
         the level is not being computed from the camera",
    );
    assert!(
        f > n,
        "the distant frame samples a LOWER level than the near one ({f:.2} vs {n:.2}), \
         which is the selection inverted",
    );
}
