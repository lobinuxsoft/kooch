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
//! Measured across the fix that made it pass:
//!
//! | | near (1.2 m) | far (90 m) |
//! |---|---|---|
//! | `half_screen_size` | 10.00 | 10.00 |
//! | `two_over_screen_size` | 3.01 | 6.99 |
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

/// A 2048x2048 one-pixel checker.
///
/// ⚠️ Big on purpose. The rig renders 200 px across, so a 256-texel
/// texture on a floor that fills the frame has a footprint under one
/// texel at any distance this scene offers — level 0 everywhere, and a
/// test with no range to measure. 2048 puts several levels between the
/// near camera and the far one.
///
/// A one-pixel checker: the finest detail a chain can lose, so
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

/// Decodes the level out of the view's colour at the centre of the
/// frame, which is where this scene always has floor.
///
/// ⚠️ The centre and not the mean. From ninety metres the floor covers a
/// handful of pixels and the rest is background, so a frame average
/// measures how much floor is on screen — which is what the first two
/// versions of this test measured, and reported as a mip level, twice.
///
/// The ramp is `base * (0.55 + 0.45 * fract(lod))` with
/// `base = (ramp, _, 1 - ramp)` and `ramp = level / 10`, so `R + B`
/// recovers the brightness factor and `R / (R + B)` recovers the level.
fn level_at_centre(image: &[u8], side: usize) -> f64 {
    let i = ((side / 2) * side + side / 2) * 4;
    let (r, b) = (f64::from(image[i]), f64::from(image[i + 2]));
    let sum = r + b;
    assert!(
        sum > 1.0,
        "the centre of the frame is black — nothing was shaded there, so there is no \
         level to read",
    );
    (r / sum) * 10.0
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
    // 🔴 The rig's OWN material guid, not a fresh one. A new material is
    // a material nothing references: the scene renders untextured, the
    // view paints its "no albedo map" magenta, and the mean of that
    // tracks how much floor is on screen — which is what the first
    // version of this test measured, and reported as a mip level.
    let material = r.material;
    {
        let pipeline = r
            .resources
            .get_mut::<MaterialPipeline>()
            .expect("the rig registers a material pipeline");
        pipeline.register_texture(&r.device, &r.queue, texture, &checker(2048));
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
fn the_mip_level_responds_to_the_camera() {
    let _gpu = gpu_lock();
    let Some(near) = mip_view_at(Vec3::new(0.0, 0.6, 1.2)) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(far) = mip_view_at(Vec3::new(0.0, 30.0, 90.0)) else {
        return;
    };

    let (n, f) = (level_at_centre(&near, 200), level_at_centre(&far, 200));
    eprintln!("mip level at the centre — near {n:.2}, far {f:.2}");
    assert!(
        (n - f).abs() > 1.0,
        "the level is the same from 1.2 m and from 90 m ({n:.2} vs {f:.2}), so it is not \
         being computed from the camera",
    );
    assert!(
        f > n,
        "the distant surface samples a LOWER level than the near one ({f:.2} vs {n:.2}), \
         which is the selection inverted",
    );
}
