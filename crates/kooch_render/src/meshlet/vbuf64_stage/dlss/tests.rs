use super::{PerfMode, perf_mode};

/// The four rungs the settings asset offers must each land on the
/// preset that shares its ratio, or a project asking for "Balanced"
/// gets DLSS reconstructing from a different number of pixels than the
/// engine rendered.
#[test]
fn the_ladder_maps_rung_for_rung() {
    let out = (1920, 1080);
    assert_eq!(perf_mode(out, out), PerfMode::Dlaa);
    assert_eq!(perf_mode((1286, 723), out), PerfMode::Quality);
    assert_eq!(perf_mode((1133, 637), out), PerfMode::Balanced);
    assert_eq!(perf_mode((960, 540), out), PerfMode::Performance);
}

#[test]
fn a_scale_below_the_ladder_is_ultra_performance() {
    assert_eq!(
        perf_mode((640, 360), (1920, 1080)),
        PerfMode::UltraPerformance
    );
}

/// A window dragged to nothing must not divide by zero on the way to a
/// preset.
#[test]
fn a_zero_output_does_not_divide() {
    assert_eq!(perf_mode((0, 0), (0, 0)), PerfMode::Dlaa);
}
