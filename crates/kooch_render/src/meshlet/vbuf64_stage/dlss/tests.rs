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

/// NVIDIA's exact ratios, which are NOT the engine's rungs: Balanced is
/// 1/1.72 where the settings asset offers 59 %, and Quality is 1/1.5
/// where it offers 67 %.
const RATIOS: [(PerfMode, f32); 5] = [
    (PerfMode::Dlaa, 1.0),
    (PerfMode::Quality, 1.5),
    (PerfMode::Balanced, 1.72),
    (PerfMode::Performance, 2.0),
    (PerfMode::UltraPerformance, 3.0),
];

/// 🔴 The anti-oscillation invariant, and the reason the stage may
/// adopt NGX's render size at all.
///
/// Once DLSS reports the size it wants, the stage reallocates to it —
/// and the preset is then derived again from that new ratio. If NGX's
/// own optimal fell in a different band from the scale that asked for
/// it, the context would be rebuilt every frame, each rebuild asking
/// for the other one's size.
#[test]
fn ngx_own_size_asks_for_the_same_preset() {
    for output in [(1610, 943), (1920, 1080), (2560, 1440), (1280, 720)] {
        for (mode, ratio) in RATIOS {
            let optimal = (
                (output.0 as f32 / ratio).round() as u32,
                (output.1 as f32 / ratio).round() as u32,
            );
            assert_eq!(
                perf_mode(optimal, output),
                mode,
                "{mode:?} at {output:?} came back as something else"
            );
        }
    }
}

/// The window that found the bug: 943 rows halved is 471 by flooring
/// and 472 by rounding, and NGX refuses the smaller one outright —
/// its minimum render resolution IS its optimal.
#[test]
fn a_half_of_an_odd_height_still_asks_for_performance() {
    let output = (1610, 943);
    assert_eq!(perf_mode((805, 471), output), PerfMode::Performance);
    assert_eq!(perf_mode((805, 472), output), PerfMode::Performance);
}
