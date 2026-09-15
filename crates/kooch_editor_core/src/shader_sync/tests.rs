use std::time::{Duration, SystemTime};

use super::ShaderSync;

#[test]
fn a_first_sighting_is_not_a_change() {
    let mut sync = ShaderSync::default();
    assert!(!sync.moved("a.shader".into(), SystemTime::UNIX_EPOCH));
}

#[test]
fn a_new_mtime_is_a_change() {
    let mut sync = ShaderSync::default();
    sync.moved("a.shader".into(), SystemTime::UNIX_EPOCH);
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    assert!(sync.moved("a.shader".into(), later));
    assert!(!sync.moved("a.shader".into(), later));
}
