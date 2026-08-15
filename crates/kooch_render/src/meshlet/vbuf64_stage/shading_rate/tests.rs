use super::*;

#[test]
fn only_half_and_two_lower_the_rate() {
    assert_eq!(parse_rate(Some("half")), Some(ShadingRate::Half));
    assert_eq!(parse_rate(Some("2")), Some(ShadingRate::Half));
    assert_eq!(parse_rate(Some("full")), Some(ShadingRate::Full));
    assert_eq!(parse_rate(Some("1")), Some(ShadingRate::Full));
    // Unrecognised reads as unset, so the project's own setting
    // stands rather than being forced to a rate nobody named.
    for raw in ["on", "HALF", "yes", ""] {
        assert_eq!(parse_rate(Some(raw)), None, "{raw}");
    }
    assert_eq!(parse_rate(None), None);
}

#[test]
fn an_odd_size_keeps_its_last_row() {
    assert_eq!(ShadingRate::Full.target_size((1281, 721)), (1281, 721));
    assert_eq!(ShadingRate::Half.target_size((1281, 721)), (641, 361));
}
