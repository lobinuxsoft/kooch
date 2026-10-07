use super::*;

/// 🔴 `!(scale > 0.0)`, not `scale <= 0.0`: a NaN passes the `<=` and would collapse every
/// handle to a point that draws as nothing and picks as nothing — the gizmo would simply vanish.
#[test]
fn an_unusable_scale_falls_back() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut set = HandleSet::default();
        set.set_scale(scale);
        assert_eq!(set.frame.scale, 1.0, "{scale}");
    }
}

/// A usable scale is taken as given: this is the one number the whole screen-sizing rests on.
#[test]
fn a_usable_scale_is_kept() {
    let mut set = HandleSet::default();
    set.set_scale(2.5);
    assert_eq!(set.frame.scale, 2.5);
}
