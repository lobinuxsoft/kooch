use super::*;

#[test]
fn the_title_is_rewritten_on_an_interval_not_every_frame() {
    let mut state = TitleMetricsState::default();
    let start = Instant::now();
    assert!(state.due(start), "the first one is due");
    assert!(!state.due(start + Duration::from_millis(100)));
    assert!(state.due(start + Duration::from_millis(300)));
}

/// Nothing happens without the environment variable — a game does not
/// get its title rewritten because it linked the engine.
#[test]
fn silent_by_default() {
    let mut resources = Resources::new();
    resources.insert(FrameMetrics::default());
    resources.insert(TitleMetricsState::default());
    // No window, no config: the early return has to fire before either
    // is reached, or this panics.
    title_metrics_system(&mut resources);
}
