use super::*;

#[test]
fn play_params_defaults_are_sensible() {
    let params = PlayParams::default();
    assert_eq!(params.volume, 1.0);
    assert_eq!(params.pitch, 1.0);
    assert!(!params.looping);
}

#[test]
fn audio_error_display_includes_message() {
    let err = AudioError::Decode("bad header".into());
    let s = format!("{err}");
    assert!(s.contains("decode"));
    assert!(s.contains("bad header"));
}
