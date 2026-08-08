use super::*;

#[test]
fn parse_round_trip() {
    for p in [
        PowerProfile::Plugged,
        PowerProfile::Balanced,
        PowerProfile::Battery,
        PowerProfile::Debug,
    ] {
        assert_eq!(PowerProfile::parse(p.as_str()), Some(p));
    }
}

#[test]
fn parse_case_insensitive() {
    assert_eq!(PowerProfile::parse("BATTERY"), Some(PowerProfile::Battery));
    assert_eq!(PowerProfile::parse("  Debug "), Some(PowerProfile::Debug));
}

#[test]
fn parse_unknown_returns_none() {
    assert!(PowerProfile::parse("turbo").is_none());
}
