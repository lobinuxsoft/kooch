use super::*;

#[test]
fn a_bare_manifest_gains_it() {
    let out = with_dev_profile("[package]\nname = \"x\"\n").expect("adds");
    assert!(out.contains(HEADER));
    assert!(out.contains("opt-level = 3"));
}

/// Adding twice would be a duplicate-key error that stops the build.
#[test]
fn an_existing_profile_is_kept() {
    let once = with_dev_profile("[package]\n").unwrap();
    assert_eq!(with_dev_profile(&once), None);
}
