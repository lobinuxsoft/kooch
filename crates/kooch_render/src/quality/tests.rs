use super::*;

/// 🔴 A technique that cannot reconstruct must not be handed a smaller frame.
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
        TemporalSettings::new(UpscaleTechnique::Taa, 50, 0, true).render_scale,
        100
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Sgsr2, 50, 0, true).render_scale,
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

/// 🔴 Sharpening is clamped at the same boundary the scale is gated at, and it is NOT gated on the
/// technique.
#[test]
fn sharpening_is_clamped_and_ungated() {
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::None, 100, 500, true).sharpening,
        100
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::None, 100, 60, true).sharpening,
        60
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Sgsr2, 50, 60, true).sharpening,
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

/// 🔴 The fragment path is refused the scale, whatever the technique.
#[test]
fn the_fragment_path_is_refused_the_scale() {
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Sgsr2, 50, 0, false).render_scale,
        100,
        "an upscaler without the compute path still got a smaller frame",
    );
    assert_eq!(
        TemporalSettings::new(UpscaleTechnique::Sgsr2, 50, 0, true).render_scale,
        50,
    );
}

mod presentation {
    use crate::quality::Presentation;

    /// With nothing set, the project's file decides — including when it
    /// says vsync off, which is the case a "no opinion" default that
    /// meant `true` would have quietly overridden.
    #[test]
    fn the_asset_decides_when_unset() {
        assert!(Presentation::resolve(true, None).vsync);
        assert!(!Presentation::resolve(false, None).vsync);
    }

    /// 🔴 The variable wins, both ways. A measurement run that asked for no vsync must get it out of
    /// a project that ships vsync on, and a run that asked for vsync back must get it out of one
    /// that ships it off — otherwise the A/B depends on which project is open.
    #[test]
    fn the_variable_outranks_the_asset() {
        assert!(!Presentation::resolve(true, Some(false)).vsync);
        assert!(Presentation::resolve(false, Some(true)).vsync);
    }

    /// Vsync on, because an uncapped editor and an uncapped handheld
    /// both burn a GPU drawing frames nobody sees.
    #[test]
    fn the_default_is_vsync() {
        assert!(Presentation::default().vsync);
    }
}
