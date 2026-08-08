use super::*;

fn stages() -> RenderStages {
    RenderStages {
        gather_ms: 1.0,
        gather: GatherStages::default(),
        ui_ms: 6.0,
        input_ms: 0.5,
        viewport_ms: 1.5,
        present_ms: 1.0,
        actions_ms: 0.0,
    }
}

#[test]
fn residual_is_what_the_stages_do_not_claim() {
    let breakdown = FrameBreakdown {
        render: stages(),
        gizmo_batch_ms: 3.0,
    };
    // 10 measured out of a 12.5 ms frame.
    assert!((breakdown.residual_ms(12.5) - 2.5).abs() < 1e-4);
}

/// The gizmo batch runs outside the span `cpu_frame_ms` covers.
/// Counting it would report a residual smaller than the truth, which
/// is the one direction a diagnostic must never err in.
#[test]
fn the_gizmo_batch_is_not_deducted_from_the_residual() {
    let with_batch = FrameBreakdown {
        render: stages(),
        gizmo_batch_ms: 3.0,
    };
    let without = FrameBreakdown {
        render: stages(),
        gizmo_batch_ms: 0.0,
    };
    assert_eq!(with_batch.residual_ms(12.5), without.residual_ms(12.5));
}

/// Float noise between two separately-read spans must not surface as
/// a negative number in a panel.
#[test]
fn a_total_below_the_stages_reads_as_zero_not_negative() {
    let breakdown = FrameBreakdown {
        render: stages(),
        gizmo_batch_ms: 0.0,
    };
    assert_eq!(breakdown.residual_ms(9.999), 0.0);
}

#[test]
fn recording_stages_leaves_the_gizmo_batch_alone() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    record_gizmo_batch_ms(&mut resources, 3.0);
    record_render_stages(&mut resources, stages());

    let breakdown = resources.get::<EditorPerfStats>().unwrap().breakdown;
    assert_eq!(
        breakdown.gizmo_batch_ms, 3.0,
        "clobbered by the render pass"
    );
    assert_eq!(breakdown.render.ui_ms, 6.0);
}

/// The sub-stages sit inside gather, so they must never sum past
/// it — a "rest of gather" that reads negative would send someone
/// looking for time that was only ever double-counted.
#[test]
fn the_gather_sub_stages_are_a_split_of_gather_not_an_addition() {
    let gather = GatherStages {
        intern_ms: 0.1,
        entities_ms: 4.0,
        archetypes_ms: 0.3,
        types_ms: 0.2,
        assets_ms: 0.4,
    };
    assert!((gather.total_ms() - 5.0).abs() < 1e-4);

    let render = RenderStages {
        gather_ms: 5.5,
        gather,
        ..stages()
    };
    // Gather's own total is what the frame residual uses; the split
    // is not added on top of it.
    let breakdown = FrameBreakdown {
        render,
        gizmo_batch_ms: 0.0,
    };
    assert!(breakdown.residual_ms(render.total_ms()) < 0.01);
}

#[test]
fn ms_since_measures_forward() {
    let start = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let elapsed = ms_since(start);
    assert!(elapsed >= 2.0, "expected ≥ 2 ms, got {elapsed}");
    assert!(elapsed < 1000.0);
}
