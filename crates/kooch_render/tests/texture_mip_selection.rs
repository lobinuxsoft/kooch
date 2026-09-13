//! Which mip level the shading path asks for (#481 follow-up).

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

/// Decodes the level out of the view's colour at the centre of the frame, which is where this scene
/// always has floor.
fn level_at_centre(image: &[u8], side: usize) -> f64 {
    level_at(image, side, side / 2, side / 2)
}

/// The level where the frame actually has surface.
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
    // 🔴 The rig's OWN material guid, not a fresh one.
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
