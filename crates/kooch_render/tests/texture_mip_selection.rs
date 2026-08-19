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

use common::lit_scene::SIZE as SIDE;
use glam::Vec3;
use kooch_core::Guid;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletDebugMode, ShadingRate};
use kooch_render::quality::UpscaleTechnique;
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
    level_at(image, side, side / 2, side / 2)
}

/// The level where the frame actually has surface.
///
/// 🔴 Reads the centroid of the lit pixels rather than a fixed
/// coordinate. A debug view is NOT upscaled — the shading writes at
/// render resolution and the tonemap copies texel for texel — so at a
/// reduced scale the picture is smaller AND in a different place, and a
/// hardcoded centre reads background. Three versions of this test read
/// the wrong pixel before this existed.
fn level_at_lit_centroid(image: &[u8], side: usize) -> f64 {
    let lit: Vec<(usize, usize)> = (0..side)
        .flat_map(|y| (0..side).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let i = (y * side + x) * 4;
            u32::from(image[i]) + u32::from(image[i + 2]) > 8
        })
        .collect();
    assert!(
        lit.len() > 32,
        "the frame has {} lit pixels, which is not a surface to measure",
        lit.len(),
    );
    let x = lit.iter().map(|(x, _)| x).sum::<usize>() / lit.len();
    let y = lit.iter().map(|(_, y)| y).sum::<usize>() / lit.len();
    level_at(image, side, x, y)
}

/// The same, at an explicit pixel.
///
/// ⚠️ Needed because a debug view is NOT upscaled: the shading writes at
/// render resolution and the tonemap copies texel for texel, so at a
/// render scale of 50 % the picture occupies the top-left quarter of the
/// frame and the rest is black. Reading the frame's centre there reads
/// the background — which the assertion below catches rather than
/// averages into a number.
fn level_at(image: &[u8], side: usize, x: usize, y: usize) -> f64 {
    let i = (y * side + x) * 4;
    let (r, b) = (f64::from(image[i]), f64::from(image[i + 2]));
    let sum = r + b;
    assert!(
        sum > 1.0,
        "the centre of the frame is black — nothing was shaded there, so there is no \
         level to read",
    );
    (r / sum) * 10.0
}

/// Renders the mip view with the camera at `eye`, with no temporal
/// technique — so no bias.
fn mip_view_at(eye: Vec3) -> Option<Vec<u8>> {
    mip_view_with(eye, UpscaleTechnique::None, 100)
}

/// The same, with a technique and a render scale.
fn mip_view_with(eye: Vec3, technique: UpscaleTechnique, scale: u32) -> Option<Vec<u8>> {
    let mut r = common::lit_scene::rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);
    r.stage.set_upscale(technique);
    r.stage.set_render_scale(scale);
    if scale != 100 {
        // Where a scale turns into textures.
        r.stage.resize(&r.device, (SIDE, SIDE));
    }

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

/// 🔴 A temporal technique buys one level of sharpness, and only then.
///
/// FSR 2 and 3 document `mipBias = log2(render / display) - 1.0`, and at
/// native resolution the first term is zero — so the whole bias is that
/// `-1`, which exists because the **jitter** resolves sub-pixel detail.
/// A history to accumulate into is what makes a sharper level come out
/// correct instead of shimmering.
///
/// Applied with no history it would be aliasing on purpose, which is why
/// the gate is on the technique and this test measures both sides of it.
#[test]
fn a_temporal_technique_sharpens_by_one_level() {
    let _gpu = gpu_lock();
    let eye = Vec3::new(0.0, 30.0, 90.0);
    let Some(plain) = mip_view_with(eye, UpscaleTechnique::None, 100) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(temporal) = mip_view_with(eye, UpscaleTechnique::Sgsr2, 100) else {
        return;
    };

    let (off, on) = (
        level_at_centre(&plain, SIDE as usize),
        level_at_centre(&temporal, SIDE as usize),
    );
    eprintln!("mip level — no technique {off:.2}, temporal at 1:1 {on:.2}");
    assert!(
        (off - on - 1.0).abs() < 0.25,
        "the bias should be exactly one level at native resolution ({off:.2} → {on:.2}); \
         anything else means the -1 is not being applied, or is being applied twice",
    );
}

/// And at a reduced scale, the frame samples as if it were native.
///
/// 🔴 The arithmetic that the first version of this test got wrong.
/// Rendering at half width **already** doubles the uv footprint per
/// pixel — fewer pixels cover the same surface — so the level climbs by
/// one on its own, with no bias at all. The bias of `log2(0.5) - 1 = -2`
/// cancels that and spends one more.
///
/// So the number to expect against a native, unbiased frame is **one
/// level sharper**, not two: the first level of bias buys back the
/// resolution the frame does not have, and only the second is new
/// detail. Measured 3.00 → 2.02.
///
/// That is the whole point of the setting. Without it every texture in
/// an upscaled frame is sampled for a resolution the output never uses,
/// and the upscaler gets blamed for softness it was handed.
#[test]
fn a_reduced_scale_samples_as_if_it_were_native() {
    let _gpu = gpu_lock();
    // Closer than the other cases: at ninety metres AND half resolution
    // the floor came back as 130 lit pixels, which is not a surface.
    let eye = Vec3::new(0.0, 8.0, 16.0);
    let Some(plain) = mip_view_with(eye, UpscaleTechnique::None, 100) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let Some(halved) = mip_view_with(eye, UpscaleTechnique::Sgsr2, 50) else {
        return;
    };

    let side = SIDE as usize;
    let (off, on) = (
        level_at_lit_centroid(&plain, side),
        level_at_lit_centroid(&halved, side),
    );
    eprintln!("mip level — native unbiased {off:.2}, half scale biased {on:.2}");
    assert!(
        (off - on - 1.0).abs() < 0.35,
        "half scale should sample one level sharper than native ({off:.2} → {on:.2}): the \
         reduced resolution costs a level on its own, the bias pays it back and spends \
         one more",
    );
}
