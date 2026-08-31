use kooch_ecs::reflect::{Reflect, ReflectValue};
use kooch_physics::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID, SHAPE_SPHERE};

use super::field_is_shown;

/// `(name, current value)` for every field, the way the Inspector
/// receives them.
fn field_values(collider: &Collider) -> Vec<(String, ReflectValue)> {
    collider
        .reflect_fields()
        .iter()
        .filter_map(|meta| {
            collider
                .reflect_get(meta.name)
                .map(|value| (meta.name.to_owned(), value))
        })
        .collect()
}

/// The field names the Inspector would actually render for a shape.
fn shown_fields(shape: u32) -> Vec<String> {
    let collider = Collider {
        shape,
        ..Default::default()
    };
    let fields = field_values(&collider);
    let metas = Some(collider.reflect_fields());
    fields
        .iter()
        .filter(|(name, _)| field_is_shown(metas, name, &fields))
        .map(|(name, _)| name.clone())
        .collect()
}

/// What was reported: a sphere showed `half_extents` and `half_height`,
/// which it ignores entirely, and they read as if they did something.
#[test]
fn a_sphere_shows_only_the_parameters_it_reads() {
    let shown = shown_fields(SHAPE_SPHERE);
    assert!(shown.contains(&"radius".to_owned()), "{shown:?}");
    assert!(shown.contains(&"center".to_owned()), "{shown:?}");
    assert!(
        !shown.contains(&"half_extents".to_owned()),
        "a sphere still offers half_extents: {shown:?}"
    );
    assert!(
        !shown.contains(&"half_height".to_owned()),
        "a sphere still offers half_height: {shown:?}"
    );
}

#[test]
fn a_cuboid_shows_its_extents_and_not_the_round_parameters() {
    let shown = shown_fields(SHAPE_CUBOID);
    assert!(shown.contains(&"half_extents".to_owned()), "{shown:?}");
    assert!(!shown.contains(&"radius".to_owned()), "{shown:?}");
    assert!(!shown.contains(&"half_height".to_owned()), "{shown:?}");
}

#[test]
fn a_capsule_shows_both_of_its_dimensions() {
    let shown = shown_fields(SHAPE_CAPSULE);
    assert!(shown.contains(&"radius".to_owned()), "{shown:?}");
    assert!(shown.contains(&"half_height".to_owned()), "{shown:?}");
    assert!(!shown.contains(&"half_extents".to_owned()), "{shown:?}");
}

/// `shape` and `center` apply to every variant, so they are never
/// filtered — a condition is opt-in per field.
#[test]
fn the_shape_selector_and_centre_always_show() {
    for shape in [SHAPE_SPHERE, SHAPE_CUBOID, SHAPE_CAPSULE, 99] {
        let shown = shown_fields(shape);
        assert!(shown.contains(&"shape".to_owned()), "shape {shape}");
        assert!(shown.contains(&"center".to_owned()), "shape {shape}");
    }
}

/// Hiding is display only. Every field is still reflected, so it is
/// still stored, still serialised, and still survives a scene
/// round-trip — the reason the storage keeps all variants side by side
/// in the first place.
#[test]
fn hidden_fields_are_still_stored_and_reflected() {
    let collider = Collider {
        shape: SHAPE_SPHERE,
        half_extents: glam::Vec3::splat(7.0),
        half_height: 3.0,
        ..Default::default()
    };
    let fields = field_values(&collider);

    // Present in reflection even though the Inspector hides them.
    let extents = fields.iter().find(|(n, _)| n == "half_extents");
    assert_eq!(
        extents.map(|(_, v)| v.clone()),
        Some(ReflectValue::Vec3(glam::Vec3::splat(7.0))),
        "a hidden field stopped being reflected"
    );
    assert!(
        !field_is_shown(Some(collider.reflect_fields()), "half_extents", &fields),
        "test is not exercising a hidden field"
    );
}

/// A `bool` works as a discriminant, and the shadow settings show only
/// the knobs the selected technique reads.
///
/// # 🔴 Two defects, one test
///
/// `integer_value` handled every integer width and **not `Bool`**, and
/// the failure was silent in the worst direction: an unreadable
/// discriminant reads as `None`, `FieldCondition::is_met(None)` reads as
/// SHOWN, so a `shown_when` pointing at a toggle hid nothing, ever. The
/// absent-field case is meant to look like a typo — an unsupported TYPE
/// looked like a working rule.
///
/// The other half is what the rule says. Which fields survive the switch
/// was read out of the code, not guessed: `shadow_cascade_texels` sizes
/// every layer of the shared atlas including the spot lights',
/// `point_shadows` is the cube budget the local lights still use, and
/// `shadows_enabled` returns the shading fully lit before either branch.
#[test]
fn the_shadow_knobs_follow_the_technique() {
    use kooch_render::settings::RenderSettings;

    let shown = |virtual_shadows: bool| -> Vec<String> {
        let settings = RenderSettings {
            virtual_shadows,
            ..Default::default()
        };
        let fields: Vec<(String, ReflectValue)> = settings
            .reflect_fields()
            .iter()
            .filter_map(|meta| {
                settings
                    .reflect_get(meta.name)
                    .map(|value| (meta.name.to_owned(), value))
            })
            .collect();
        let metas = Some(settings.reflect_fields());
        fields
            .iter()
            .filter(|(name, _)| field_is_shown(metas, name, &fields))
            .map(|(name, _)| name.clone())
            .collect()
    };

    let pages = shown(true);
    let cascades = shown(false);
    let has = |list: &[String], name: &str| list.iter().any(|n| n == name);

    // The half that proves a bool reaches the condition at all: if it
    // did not, both lists would be identical and every assert below
    // would pass for the wrong reason.
    assert_ne!(
        pages, cascades,
        "the technique toggle changed nothing; a bool discriminant is \
         being read as absent and every `shown_when` on it is inert"
    );

    for field in ["shadow_density", "shadow_pool_pages"] {
        assert!(has(&pages, field), "{field} is missing with pages on");
        assert!(!has(&cascades, field), "{field} shows with pages off");
    }
    // 🔴 Inert with the pages on, and hidden for it. The virtual shadow
    // map REPLACES the cascades rather than blending with them —
    // `inti_shadow` returns to `inti_page_shadow` before it picks one —
    // and the cube path is skipped wholesale: `draw_cascades` is
    // `cascades_enabled && !virtual_pages`, the point and spot caster
    // lists come back empty, and `classic_shadow_alloc` shrinks the
    // classic atlas to 256 texels and one cube of 16.
    //
    // `shadow_cascade_texels` and `point_shadows` were in the list below
    // — asserted visible in BOTH modes — until 89c5e71e hid them on
    // purpose and left this test asserting the old answer. They size an
    // atlas nothing draws into and nothing samples.
    for field in [
        "shadow_distance",
        "sun_softness",
        "shadow_first_cascade_distance",
        "shadow_cascade_texels",
        "point_shadows",
    ] {
        assert!(has(&cascades, field), "{field} is missing with pages off");
        assert!(!has(&pages, field), "{field} shows with pages on");
    }
    // The two that mean something under either technique: one is the
    // master switch and the other picks between them.
    for field in ["shadows_enabled", "virtual_shadows"] {
        assert!(has(&pages, field), "{field} vanished with pages on");
        assert!(has(&cascades, field), "{field} vanished with pages off");
    }
}
