use super::*;

#[test]
fn an_absurd_cascade_size_is_clamped_rather_than_allocated() {
    let settings = ShadowSettings {
        cascade_texels: 65_536,
        ..Default::default()
    };
    assert_eq!(settings.clamped_texels(), 4096);
    assert!(settings.clamped_texels() * 2 <= 8192);
}

/// Unity 150, Unreal 200, Godot 100, Bevy 100. A scene, not a
/// planet — and the number that decides how much of the atlas is
/// spent on ground the player is nowhere near.
#[test]
fn the_default_covers_a_scene_rather_than_a_planet() {
    assert_eq!(ShadowSettings::default().max_distance, 100.0);
    assert_eq!(ShadowSettings::default().first_cascade_distance, 10.0);
}
