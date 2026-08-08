use super::*;

#[test]
fn stats_start_empty_and_saturate_rather_than_wrap() {
    let cell = CallStatsCell::default();
    assert_eq!(cell.load(), CallStats::default());

    cell.store(
        Duration::from_secs(u64::MAX / 2),
        Duration::ZERO,
        usize::MAX,
    );
    let stats = cell.load();
    assert_eq!(
        stats.transport_us,
        u32::MAX,
        "a stuck call must read as enormous"
    );
    assert_eq!(stats.response_bytes, u32::MAX);
}
