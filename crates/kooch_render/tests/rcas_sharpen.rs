//! RCAS, the pass that ends the frame (#481, step 5).

mod common;

/// Serialised for the reason the other GPU binaries are: `common` hands
/// every case the same device, and concurrent submission against it
/// segfaults radv rather than failing a case.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

use common::lit_scene::{SIZE, rig};

/// One frame of the lit scene, sharpened by `percent`.
fn frame(percent: u32) -> Option<Vec<u8>> {
    let mut r = rig(3, true)?;
    assert!(r.stage.set_compute_shading(true) > 0);
    assert!(
        r.stage.set_sharpening(percent) > 0,
        "no view took the amount — every assertion below would be vacuous",
    );
    r.stage
        .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
    Some(common::read_rgba8(
        &r.device,
        &r.queue,
        r.stage.color_texture(),
    ))
}

/// The mean absolute difference between horizontally adjacent pixels.
fn mean_gradient(image: &[u8]) -> f64 {
    let side = SIZE as usize;
    let mut sum = 0u64;
    let mut count = 0u64;
    for y in 0..side {
        for x in 1..side {
            let a = (y * side + x) * 4;
            let b = (y * side + x - 1) * 4;
            for c in 0..3 {
                sum += image[a + c].abs_diff(image[b + c]) as u64;
                count += 1;
            }
        }
    }
    sum as f64 / count as f64
}

/// The same measure as [`mean_gradient`], restricted to one row.
fn row_gradient(image: &[u8], y: usize) -> f64 {
    let side = SIZE as usize;
    let mut sum = 0u64;
    let mut count = 0u64;
    for x in 1..side {
        let a = (y * side + x) * 4;
        let b = (y * side + x - 1) * 4;
        for c in 0..3 {
            sum += image[a + c].abs_diff(image[b + c]) as u64;
            count += 1;
        }
    }
    sum as f64 / count as f64
}

/// 🔴 The pass raises local contrast, which is the only thing it claims.
#[test]
fn sharpening_raises_local_contrast() {
    let _gpu = gpu_lock();
    let Some(plain) = frame(0) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    let Some(sharp) = frame(100) else { return };

    let (a, b) = (mean_gradient(&plain), mean_gradient(&sharp));
    eprintln!("mean gradient — off {a:.4}, at 100 % {b:.4}");
    assert!(
        b > a * 1.05,
        "sharpening moved the local contrast by less than 5 % ({a:.4} → {b:.4}), which \
         is what an unwired pass, a zeroed amount and a blur all look like",
    );
}

/// And the amount reaches the shader, at the amounts an author sets.
#[test]
fn the_amount_reaches_the_shader() {
    let _gpu = gpu_lock();
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    assert!(r.stage.set_compute_shading(true) > 0);

    let mut shot = |percent: u32, r: &mut common::lit_scene::Rig| -> Vec<u8> {
        assert!(r.stage.set_sharpening(percent) > 0);
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        common::read_rgba8(&r.device, &r.queue, r.stage.color_texture())
    };

    // ⚠️ Settled first, rather than trusting frame zero. The opening frames of a rig upload the
    // meshlets and the material textures and come back black or half-drawn — which failed this file
    // in a batch while passing on its own, twice.
    let mut first = shot(0, &mut r);
    let mut settled = false;
    for _ in 0..8 {
        let again = shot(0, &mut r);
        if again == first {
            settled = true;
            break;
        }
        first = again;
    }
    assert!(
        settled,
        "the rig never produced two identical frames in a row, so nothing below could \
         distinguish the setting from the noise",
    );

    let sharp = shot(60, &mut r);
    assert_ne!(
        first, sharp,
        "60 % produced a bit-identical image, so the amount is being dropped somewhere \
         between the settings asset and the uniform",
    );
}

/// 🔴 The limiter limits: no pixel is moved far.
#[test]
fn the_limiter_limits() {
    let _gpu = gpu_lock();
    let Some(plain) = frame(0) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let Some(sharp) = frame(100) else { return };

    let mut diffs: Vec<u8> = plain
        .chunks_exact(4)
        .zip(sharp.chunks_exact(4))
        .flat_map(|(a, b)| {
            [
                a[0].abs_diff(b[0]),
                a[1].abs_diff(b[1]),
                a[2].abs_diff(b[2]),
            ]
        })
        .collect();
    diffs.sort_unstable();
    let p999 = diffs[diffs.len() * 999 / 1000] as f64;
    let worst = *diffs.last().unwrap() as f64;
    eprintln!("per-channel change — p99.9 {p999}, worst {worst}");
    assert!(
        p999 <= 24.0,
        "the strongest 0.1 % of the frame moved by {p999} of 255, which is a halo \
         rather than an edge — the solved lobe is not being capped",
    );
}

/// 🔴 The last row is sharpened like every other row.
#[test]
fn the_last_row_is_sharpened_too() {
    let _gpu = gpu_lock();
    let Some(plain) = frame(0) else {
        eprintln!("no adapter; skipping");
        return;
    };
    let Some(sharp) = frame(100) else { return };

    let last = (SIZE - 1) as usize;
    let (a, b) = (row_gradient(&plain, last), row_gradient(&sharp, last));
    eprintln!("bottom-row gradient — off {a:.4}, at 100 % {b:.4}");
    assert!(
        b > a * 1.05,
        "the last row's local contrast barely moved ({a:.4} → {b:.4}) while the frame's \
         did, so the row is reading a neighbour that does not exist",
    );
}
