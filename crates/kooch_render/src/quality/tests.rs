use super::*;

/// 🔴 A technique that cannot reconstruct must not be handed a
/// smaller frame.
///
/// `None` and `TAA` both resolve at render resolution, so a scale
/// under 100 there is a smaller image blown up by the blit: softer,
/// and the speed goes back out through the upscale it cannot do.
/// That is the classic way this setting earns a bad name, and it is
/// refused here rather than documented as a footgun.
#[test]
fn only_an_upscaler_renders_smaller() {
    let out = (1920, 1080);
    assert_eq!(UpscaleTechnique::None.render_size(out, 50), out);
    assert_eq!(UpscaleTechnique::Taa.render_size(out, 50), out);
    assert_eq!(UpscaleTechnique::Sgsr2.render_size(out, 50), (960, 540));
}

/// And the gate is applied once, at the settings boundary, so
/// nothing downstream has to remember to ask.
#[test]
fn the_settings_clamp_the_scale() {
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Taa, 50, 0).render_scale,
        100
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Sgsr2, 50, 0).render_scale,
        50
    );
}

/// A window dragged to nothing must not ask wgpu for a zero-sized
/// texture, which it rejects outright — the frame after a minimise
/// would fail rather than render nothing.
#[test]
fn a_tiny_window_stays_renderable() {
    assert_eq!(UpscaleTechnique::Sgsr2.render_size((1, 1), 50), (1, 1));
    assert_eq!(UpscaleTechnique::Sgsr2.render_size((0, 0), 50), (1, 1));
}

/// 🔴 Sharpening is clamped at the same boundary the scale is gated
/// at, and it is NOT gated on the technique.
///
/// The clamp matters because the amount multiplies a limiter that
/// upstream measured as the edge of natural results: 500 % is five
/// times past it, which is a halo around every edge in the frame,
/// from one typo in a text file. And the absence of a gate is
/// deliberate — a native frame is allowed to ask for a little.
#[test]
fn sharpening_is_clamped_and_ungated() {
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::None, 100, 500).sharpening,
        100
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::None, 100, 60).sharpening,
        60
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Sgsr2, 50, 60).sharpening,
        60
    );
}

/// 100 is the identity, and it is what every capture on record was
/// taken at.
#[test]
fn native_scale_changes_nothing() {
    let out = (1280, 720);
    assert_eq!(UpscaleTechnique::Sgsr2.render_size(out, 100), out);
    assert_eq!(UpscaleTechnique::Sgsr2.render_size(out, 200), out);
}
