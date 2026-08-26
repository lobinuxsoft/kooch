use super::*;

/// A tool that is on by default is clutter, and this one costs frame
/// time to produce.
#[test]
fn nothing_is_enabled_by_default() {
    assert!(!DebugCategories::default().any());
}

#[test]
fn all_enables_every_category() {
    let all = DebugCategories::all();
    assert!(all.any());
    for enabled in [
        all.collider_shapes,
        all.contacts,
        all.joints,
        all.collider_aabbs,
        all.body_axes,
    ] {
        assert!(enabled, "a category is missing from `all`");
    }
}

/// `any` has to notice each switch on its own, or turning one on
/// silently draws nothing.
#[test]
fn any_notices_each_category_alone() {
    let cases = [
        DebugCategories {
            collider_shapes: true,
            ..Default::default()
        },
        DebugCategories {
            contacts: true,
            ..Default::default()
        },
        DebugCategories {
            joints: true,
            ..Default::default()
        },
        DebugCategories {
            collider_aabbs: true,
            ..Default::default()
        },
        DebugCategories {
            body_axes: true,
            ..Default::default()
        },
    ];
    for case in cases {
        assert!(case.any(), "{case:?} reports nothing enabled");
    }
}
