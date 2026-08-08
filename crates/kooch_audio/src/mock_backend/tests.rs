use super::*;

fn fake_bytes() -> &'static [u8] {
    b"fake-audio-bytes"
}

#[test]
fn load_unload_round_trip() {
    let mut backend = MockAudioBackend::new();
    let handle = backend.load_sound(fake_bytes()).unwrap();
    assert!(backend.contains_sound(handle));
    assert_eq!(backend.sound_count(), 1);

    backend.unload_sound(handle);
    assert!(!backend.contains_sound(handle));
    assert_eq!(backend.sound_count(), 0);
}

#[test]
fn play_returns_distinct_instances() {
    let mut backend = MockAudioBackend::new();
    let sound = backend.load_sound(fake_bytes()).unwrap();
    let inst1 = backend.play(sound, PlayParams::default()).unwrap();
    let inst2 = backend.play(sound, PlayParams::default()).unwrap();
    assert_ne!(inst1, inst2);
    assert_eq!(backend.instance_count(), 2);
}

#[test]
fn play_with_stale_sound_handle_errs() {
    let mut backend = MockAudioBackend::new();
    let sound = backend.load_sound(fake_bytes()).unwrap();
    backend.unload_sound(sound);
    let err = backend.play(sound, PlayParams::default()).unwrap_err();
    assert!(matches!(err, AudioError::SoundNotFound));
}

#[test]
fn stop_marks_instance_stopped_but_keeps_handle() {
    let mut backend = MockAudioBackend::new();
    let sound = backend.load_sound(fake_bytes()).unwrap();
    let inst = backend.play(sound, PlayParams::default()).unwrap();
    backend.stop(inst);
    assert_eq!(backend.instance_stopped(inst), Some(true));
    // contains_instance still true — backends prune lazily
    assert!(backend.contains_instance(inst));
}

#[test]
fn play_with_volume_persists_in_instance() {
    let mut backend = MockAudioBackend::new();
    let sound = backend.load_sound(fake_bytes()).unwrap();
    let mut params = PlayParams::default();
    params.volume = 0.25;
    let inst = backend.play(sound, params).unwrap();
    assert_eq!(backend.instance_volume(inst), Some(0.25));
}

#[test]
fn set_instance_volume_updates_value() {
    let mut backend = MockAudioBackend::new();
    let sound = backend.load_sound(fake_bytes()).unwrap();
    let inst = backend.play(sound, PlayParams::default()).unwrap();
    backend.set_instance_volume(inst, 0.5);
    assert_eq!(backend.instance_volume(inst), Some(0.5));
}

#[test]
fn set_master_volume_round_trips() {
    let mut backend = MockAudioBackend::new();
    assert_eq!(backend.master_volume(), 1.0);
    backend.set_master_volume(0.7);
    assert!((backend.master_volume() - 0.7).abs() < 1e-6);
}

#[test]
fn set_listener_round_trips() {
    let mut backend = MockAudioBackend::new();
    let pos = Vec3::new(1.0, 2.0, 3.0);
    let rot = Quat::from_rotation_y(std::f32::consts::PI);
    backend.set_listener(pos, rot);
    let (got_pos, got_rot) = backend.listener();
    assert_eq!(got_pos, pos);
    assert!((got_rot.dot(rot)).abs() > 0.999);
}

#[test]
fn stale_instance_setters_are_silent_noops() {
    let mut backend = MockAudioBackend::new();
    let sound = backend.load_sound(fake_bytes()).unwrap();
    let inst = backend.play(sound, PlayParams::default()).unwrap();
    // We can't really "remove" instances directly without a remove
    // method on the trait, but slotmap defaults to keep handles
    // until lazy prune. Build a synthetic stale handle by removing
    // via slotmap directly:
    backend.instances.remove(inst);
    // Setters should not panic on stale handle.
    backend.set_instance_volume(inst, 0.0);
    backend.stop(inst);
    assert!(!backend.contains_instance(inst));
}

#[test]
fn sound_bytes_inspector_returns_payload() {
    let mut backend = MockAudioBackend::new();
    let h = backend.load_sound(fake_bytes()).unwrap();
    assert_eq!(backend.sound_bytes(h), Some(fake_bytes()));
}
