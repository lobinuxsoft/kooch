//! Anisotropic filtering, measured where it acts (#881).
//!
//! An ordinary filter has one level for a footprint that is long and
//! thin: it takes the LONG axis, picks a level that would not alias
//! there, and blurs the short axis by the same amount. That is why a
//! tiled floor softens towards the horizon while a wall facing the
//! camera stays sharp. Anisotropic filtering takes several samples along
//! the long axis instead of one coarse one.
//!
//! So the test looks at a floor at a grazing angle and asks whether
//! there is MORE DETAIL, which is the only thing the setting claims.
//!
//! Run with:
//!   cargo test -p kooch_render --test texture_anisotropy

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
use kooch_render::meshlet::ShadingRate;
use kooch_render::quality::ShadingSettings;
use kooch_render::texture::{Image, ImageFormat};

/// A one-pixel checker: the finest detail a filter can lose.
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

/// Mean absolute difference between horizontally adjacent pixels — how
/// much detail survived to the screen.
fn detail(image: &[u8]) -> f64 {
    let side = SIDE as usize;
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in 0..side {
        for x in 1..side {
            let (a, b) = ((y * side + x) * 4, (y * side + x - 1) * 4);
            sum += u64::from(image[a].abs_diff(image[b]));
            count += 1;
        }
    }
    sum as f64 / count as f64
}

/// The floor at a grazing angle, with `samples` of anisotropy.
fn grazing_floor(samples: u16) -> Option<Vec<u8>> {
    let mut r = common::lit_scene::rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    r.stage.set_shading_rate(ShadingRate::Full);

    let texture = Guid::new_v4();
    {
        let pipeline = r.resources.get_mut::<MaterialPipeline>()?;
        pipeline.register_texture(&r.device, &r.queue, texture, &checker(1024));
        pipeline.register(
            &r.queue,
            r.material,
            // Tiled, so the floor carries high frequency all the way to
            // the horizon — untiled, one checker stretched over twenty
            // metres has nothing for a filter to lose.
            &Material::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.9, 0.0)
                .with_albedo(texture)
                .with_uv([8.0, 8.0], [0.0, 0.0]),
        );
        // ⚠️ Directly, not through `ShadingSettings`: this rig registers
        // its material by hand, so the texture sync that reads the
        // project's settings has no snapshots to run on. The setting's
        // journey from the asset is a separate claim, tested separately.
        pipeline.set_anisotropy(&r.device, samples);
    }
    r.resources.insert(ShadingSettings {
        compute: true,
        rate: ShadingRate::Full,
        anisotropy: samples,
    });
    // Low and looking along the floor: the footprint is long and thin,
    // which is the only place this setting does anything.
    r.camera = ViewCamera::looking_at(Vec3::new(0.0, 0.35, 6.0), Vec3::new(0.0, 0.3, -20.0));

    let mut last = Vec::new();
    for _ in 0..3 {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        last = common::read_rgba8(&r.device, &r.queue, r.stage.color_texture());
    }
    Some(last)
}

/// 🔴 More samples along the long axis means more detail survives.
///
/// The claim, and the only one: at a grazing angle an isotropic filter
/// blurs the short axis to whatever the long axis needed, and this is
/// what stops it. If the two frames match, either the sampler was not
/// rebuilt or the setting never reached it.
#[test]
fn anisotropy_keeps_detail_at_a_grazing_angle() {
    let _gpu = gpu_lock();
    let Some(off) = grazing_floor(1) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(on) = grazing_floor(16) else { return };

    let (a, b) = (detail(&off), detail(&on));
    eprintln!("detail on a grazing floor — off {a:.3}, 16x {b:.3}");
    assert!(
        b > a * 1.02,
        "16x anisotropy kept no more detail than none ({a:.3} → {b:.3}); either the \
         sampler was not rebuilt or the setting never reached it",
    );
}
