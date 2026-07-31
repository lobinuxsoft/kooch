//! [`MockAudioBackend`] — headless backend for tests + tooling.
//!
//! Tracks sound + instance lifecycle in slotmaps without touching any
//! real audio API. Trait conformance + game-side audio logic can be
//! exercised in CI without an audio device.

use glam::{Quat, Vec3};
use slotmap::SlotMap;

use crate::backend::{
    AudioBackend, AudioError, InstanceHandle, PlayParams, SoundHandle,
};

#[derive(Clone)]
struct MockSound {
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct MockInstance {
    sound: SoundHandle,
    volume: f32,
    looping: bool,
    pitch: f32,
    stopped: bool,
}

/// Test-friendly backend.
///
/// Honours every [`AudioBackend`] method with state-tracking semantics:
/// lifecycle (load / unload / play / stop), volume changes, listener
/// updates. No actual playback occurs — `set_master_volume` /
/// `set_listener` simply update internal fields the test can read back.
#[derive(Default)]
pub struct MockAudioBackend {
    sounds: SlotMap<SoundHandle, MockSound>,
    instances: SlotMap<InstanceHandle, MockInstance>,
    master_volume: f32,
    listener_pos: Vec3,
    listener_rot: Quat,
}

impl MockAudioBackend {
    pub fn new() -> Self {
        Self {
            sounds: SlotMap::with_key(),
            instances: SlotMap::with_key(),
            master_volume: 1.0,
            listener_pos: Vec3::ZERO,
            listener_rot: Quat::IDENTITY,
        }
    }

    /// Returns the recorded volume of an instance, or `None` for stale.
    pub fn instance_volume(&self, instance: InstanceHandle) -> Option<f32> {
        self.instances.get(instance).map(|i| i.volume)
    }

    /// Returns the master volume.
    pub fn master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Returns whether an instance has been explicitly stopped.
    pub fn instance_stopped(&self, instance: InstanceHandle) -> Option<bool> {
        self.instances.get(instance).map(|i| i.stopped)
    }

    /// Returns the original byte payload for the sound (test inspector).
    pub fn sound_bytes(&self, sound: SoundHandle) -> Option<&[u8]> {
        self.sounds.get(sound).map(|s| s.bytes.as_slice())
    }
}

impl AudioBackend for MockAudioBackend {
    fn load_sound(&mut self, bytes: &[u8]) -> Result<SoundHandle, AudioError> {
        Ok(self.sounds.insert(MockSound {
            bytes: bytes.to_vec(),
        }))
    }

    fn unload_sound(&mut self, handle: SoundHandle) {
        self.sounds.remove(handle);
    }

    fn sound_count(&self) -> usize {
        self.sounds.len()
    }

    fn contains_sound(&self, handle: SoundHandle) -> bool {
        self.sounds.contains_key(handle)
    }

    fn play(
        &mut self,
        sound: SoundHandle,
        params: PlayParams,
    ) -> Result<InstanceHandle, AudioError> {
        if !self.sounds.contains_key(sound) {
            return Err(AudioError::SoundNotFound);
        }
        Ok(self.instances.insert(MockInstance {
            sound,
            volume: params.volume,
            looping: params.looping,
            pitch: params.pitch,
            stopped: false,
        }))
    }

    fn stop(&mut self, instance: InstanceHandle) {
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.stopped = true;
        }
    }

    fn instance_count(&self) -> usize {
        self.instances.len()
    }

    fn contains_instance(&self, handle: InstanceHandle) -> bool {
        self.instances.contains_key(handle)
    }

    fn set_instance_volume(&mut self, instance: InstanceHandle, volume: f32) {
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.volume = volume;
        }
    }

    fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume;
    }

    fn set_listener(&mut self, position: Vec3, rotation: Quat) {
        self.listener_pos = position;
        self.listener_rot = rotation;
    }

    fn listener(&self) -> (Vec3, Quat) {
        (self.listener_pos, self.listener_rot)
    }
}

#[cfg(test)]
mod tests {
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
}
