use super::*;

#[test]
fn membership_carries_the_scene_it_was_given() {
    let scene = Guid::new_v4();
    assert_eq!(SceneMember::new(scene).scene, scene);
}

/// Two scenes must be distinguishable, which is the entire job.
#[test]
fn two_scenes_are_not_equal() {
    assert_ne!(
        SceneMember::new(Guid::new_v4()),
        SceneMember::new(Guid::new_v4())
    );
}
