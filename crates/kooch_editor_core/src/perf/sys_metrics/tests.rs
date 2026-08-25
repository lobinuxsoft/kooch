use super::*;

/// The whole point: a closed section costs nothing (#703).
#[test]
fn a_hidden_section_is_never_polled() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    resources.insert(super::super::HudVisibility {
        panel_visible: false,
        system_section: true,
        shadow_pages_window: false,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);

    assert!(
        resources
            .get::<SysMetricsState>()
            .unwrap()
            .last_refresh
            .is_none(),
        "the OS was asked about a section nobody can see",
    );
}

/// Both halves of the question. The section being open inside a
/// closed panel is not the same as being on screen.
#[test]
fn a_collapsed_section_inside_a_visible_panel_is_also_skipped() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: false,
        shadow_pages_window: false,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);

    assert!(
        resources
            .get::<SysMetricsState>()
            .unwrap()
            .last_refresh
            .is_none(),
    );
}

/// Reopening re-establishes the baseline instead of publishing the
/// average CPU over however long the section was closed as if it
/// were this moment's.
#[test]
fn reopening_the_section_does_not_report_the_idle_average_as_current() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: true,
        ..Default::default()
    });
    // Warm up: two refreshes establish a real delta. The panel flag is
    // re-asserted before each call the way the UI does every frame —
    // the system clears it after reading.
    sys_metrics_system(&mut resources);
    resources.get_mut::<SysMetricsState>().unwrap().last_refresh = None;
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: true,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);
    assert!(resources.get::<SysMetricsState>().unwrap().samples_taken >= 2);

    // Hidden, then shown again.
    resources.insert(super::super::HudVisibility {
        panel_visible: false,
        system_section: true,
        shadow_pages_window: false,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: true,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);

    assert!(
        resources.get::<SysMetricsState>().unwrap().samples_taken < 2,
        "the first reading after reopening must be a baseline, not a published sample",
    );
}

#[test]
fn first_call_populates_ram_eventually() {
    // Sysinfo's first cpu_usage() reading is typically 0.0 (no
    // delta yet), but RAM is available immediately. Verify that
    // running the system at least populates the RAM field.
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: true,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);
    let stats = resources.get::<EditorPerfStats>().unwrap();
    assert!(
        stats.ram_rss_mb > 0,
        "expected the editor process to report some RSS, got {}",
        stats.ram_rss_mb
    );
}

#[test]
fn second_call_inside_refresh_interval_is_a_noop() {
    // Two calls back-to-back: only the first should hit the OS.
    // We can't observe the syscall count directly, but we CAN
    // observe that `last_refresh` doesn't move on the second
    // call.
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: true,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);
    let first = resources
        .get::<SysMetricsState>()
        .unwrap()
        .last_refresh
        .unwrap();
    sys_metrics_system(&mut resources);
    let second = resources
        .get::<SysMetricsState>()
        .unwrap()
        .last_refresh
        .unwrap();
    assert_eq!(first, second, "back-to-back call must not refresh again");
}

#[test]
fn first_sample_does_not_overwrite_cpu_percent() {
    // sysinfo's first refresh always reports cpu_usage = 0.0
    // because there's no prior sample to delta against. If the
    // HUD reads this value it will display a stuck "0.0 %"
    // until enough idle time passes for a second refresh — the
    // user reported exactly this. Verify the first refresh does
    // not overwrite an existing non-zero value.
    let mut resources = Resources::default();
    let mut seeded_stats = EditorPerfStats::default();
    seeded_stats.cpu_percent = 42.0; // simulate prior reading
    resources.insert(seeded_stats);
    resources.insert(super::super::HudVisibility {
        panel_visible: true,
        system_section: true,
        ..Default::default()
    });
    sys_metrics_system(&mut resources);
    let stats = resources.get::<EditorPerfStats>().unwrap();
    assert_eq!(
        stats.cpu_percent, 42.0,
        "first sample must NOT overwrite a non-zero cpu_percent (sysinfo returns 0 \
             on the first refresh; only the second has a real delta)"
    );
    // RAM, in contrast, is a snapshot — should populate even
    // from the first sample.
    assert!(
        stats.ram_rss_mb > 0,
        "RAM must populate from the very first sample"
    );
}
