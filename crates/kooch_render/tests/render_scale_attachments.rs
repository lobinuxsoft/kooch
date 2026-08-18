//! Attachment sizes under a render scale below 100.
//!
//! Reported from the editor at a panel of 1023x816 and Performance
//! (50 %): wgpu refused the frame outright.
//!
//! ```text
//! Attachments have differing sizes: the depth attachment's texture view
//! has extent (511, 408, 1) but is followed by the color attachment at
//! index 0's texture view which has (1023, 816, 1)
//! ```
//!
//! The whole pass is discarded when that happens, so everything judged
//! on such a frame — a material, a mip chain, a sharpening amount — is
//! being judged against a picture the driver threw away.
//!
//! Run with:
//!   cargo test -p kooch_render --test render_scale_attachments

mod common;

/// Serialised: `common` hands every case the same device, and
/// concurrent submission against it segfaults radv.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

use common::lit_scene::rig;
use kooch_render::quality::UpscaleTechnique;

/// Renders one frame at `size` with the scale applied, on both shading
/// paths. A validation error aborts the process through the device's
/// error handler, which is what makes this a test at all.
fn render_at(size: (u32, u32), compute: bool) -> bool {
    let Some(mut r) = rig(3, true) else {
        return false;
    };
    r.stage.set_compute_shading(compute);
    assert!(r.stage.set_upscale(UpscaleTechnique::Sgsr2) > 0);
    r.stage.set_render_scale(50);
    r.stage.resize(&r.device, size);
    r.stage
        .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
    let _ = r.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    true
}

/// 🔴 An odd-sized panel halves to a size the other attachments must
/// follow.
///
/// 1023 renders at 511 and not at 511.5, so every target that is sized
/// from the render resolution lands on 511 while anything still sized
/// from the window stays at 1023. An even panel hides this: 1024 halves
/// to 512 and a target that took the wrong size is merely twice as big,
/// which wgpu accepts as long as the two attachments in one pass agree.
#[test]
fn an_odd_panel_keeps_every_attachment_in_step() {
    let _gpu = gpu_lock();
    if !render_at((1023, 816), true) {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
    }
}

/// The same on the fragment shading path, which has passes of its own.
#[test]
fn the_fragment_path_agrees_too() {
    let _gpu = gpu_lock();
    if !render_at((1023, 816), false) {
        eprintln!("no adapter; skipping");
    }
}

/// And an even panel, which is what every test before this one used —
/// so a regression here says the break is not about parity at all.
#[test]
fn an_even_panel_still_works() {
    let _gpu = gpu_lock();
    if !render_at((1024, 816), true) {
        eprintln!("no adapter; skipping");
    }
}

/// 🔴 The frame where the two settings disagree.
///
/// This is the one the editor actually hits, and the gate at the
/// settings boundary does not catch it: the scale and the shading path
/// arrive on different frames. `resize_view` allocates from the scale it
/// can see; `render` applies the path it was given. In between there is
/// a frame whose targets belong to one answer and whose shader belongs
/// to the other — reported from a running editor **after** the boundary
/// gate had shipped, which is how this test came to exist.
///
/// Reproduced by doing exactly that: allocate at a reduced size with the
/// compute path on, then switch the path off without resizing.
#[test]
fn a_path_switch_between_frames_does_not_take_the_frame_down() {
    let _gpu = gpu_lock();
    let Some(mut r) = rig(3, true) else {
        eprintln!("no adapter; skipping");
        return;
    };
    assert!(r.stage.set_compute_shading(true) > 0);
    assert!(r.stage.set_upscale(UpscaleTechnique::Sgsr2) > 0);
    r.stage.set_render_scale(50);
    r.stage.resize(&r.device, (1023, 816));

    // The frame the editor produces: the path changed, the attachments
    // have not been rebuilt for it yet.
    assert!(r.stage.set_compute_shading(false) > 0);
    r.stage
        .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
    let _ = r.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
}
