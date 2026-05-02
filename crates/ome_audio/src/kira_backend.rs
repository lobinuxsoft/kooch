//! [`KiraBackend`] — production [`AudioBackend`] backed by Kira 0.9.
//!
//! Wraps `kira::manager::AudioManager` and tracks sound + instance
//! handles via slotmap. Decode happens via Kira's bundled decoders
//! (mp3 / ogg / flac / wav) inside `load_sound`. Production builds use
//! `kira::manager::backend::DefaultBackend` (cpal); tests use
//! [`KiraBackend::new_with_mock_backend`] to bypass cpal init failures
//! on headless systems.

use glam::{Quat, Vec3};
use slotmap::SlotMap;
use std::io::Cursor;

use kira::manager::backend::{Backend as KiraBackendTrait, DefaultBackend};
use kira::manager::{AudioManager, AudioManagerSettings};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::tween::Tween;

use crate::backend::{
    AudioBackend, AudioError, InstanceHandle, PlayParams, SoundHandle,
};

/// Audio backend wrapping `kira::manager::AudioManager`.
///
/// Generic over Kira's `Backend` trait so production code uses
/// [`DefaultBackend`] (cpal-driven) and tests / headless tools use
/// [`kira::manager::backend::mock::MockBackend`].
pub struct KiraBackend<B: KiraBackendTrait = DefaultBackend> {
    manager: AudioManager<B>,
    sounds: SlotMap<SoundHandle, StaticSoundData>,
    instances: SlotMap<InstanceHandle, StaticSoundHandle>,
    listener_pos: Vec3,
    listener_rot: Quat,
}

impl KiraBackend<DefaultBackend> {
    /// Production constructor — opens the default audio device via cpal.
    /// Returns `AudioError::BackendInit` when no audio device is
    /// available (headless CI, server-only builds, etc.).
    pub fn new() -> Result<Self, AudioError> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| AudioError::BackendInit(format!("{e}")))?;
        Ok(Self::from_manager(manager))
    }
}

impl<B: KiraBackendTrait> KiraBackend<B> {
    /// Builds the backend around an already-constructed
    /// [`AudioManager`]. Lets tests inject a mock backend without going
    /// through cpal.
    pub fn from_manager(manager: AudioManager<B>) -> Self {
        Self {
            manager,
            sounds: SlotMap::with_key(),
            instances: SlotMap::with_key(),
            listener_pos: Vec3::ZERO,
            listener_rot: Quat::IDENTITY,
        }
    }
}

#[cfg(test)]
impl KiraBackend<kira::manager::backend::mock::MockBackend> {
    /// Test-only constructor wired to Kira's MockBackend so unit tests
    /// run without an audio device. Audio output is not produced — only
    /// the management API is exercised.
    pub fn new_with_mock_backend() -> Result<Self, AudioError> {
        let manager = AudioManager::<kira::manager::backend::mock::MockBackend>::new(
            AudioManagerSettings::default(),
        )
        .map_err(|e| AudioError::BackendInit(format!("{e:?}")))?;
        Ok(Self::from_manager(manager))
    }
}

impl<B: KiraBackendTrait> AudioBackend for KiraBackend<B>
where
    B: Send + Sync + 'static,
{
    fn load_sound(&mut self, bytes: &[u8]) -> Result<SoundHandle, AudioError> {
        let cursor = Cursor::new(bytes.to_vec());
        let data = StaticSoundData::from_cursor(cursor)
            .map_err(|e| AudioError::Decode(format!("{e}")))?;
        Ok(self.sounds.insert(data))
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
        let data = self.sounds.get(sound).ok_or(AudioError::SoundNotFound)?;
        let mut play_data = data.clone();
        if params.looping {
            // Kira's StaticSoundSettings.loop_region exposes looping;
            // for PR-1 we just toggle the simplest "loop entire sound"
            // form via the slice helpers.
            play_data = play_data.loop_region(..);
        }
        if (params.volume - 1.0).abs() > 1e-6 {
            play_data = play_data.volume(params.volume as f64);
        }
        if (params.pitch - 1.0).abs() > 1e-6 {
            play_data = play_data.playback_rate(params.pitch as f64);
        }
        let handle = self
            .manager
            .play(play_data)
            .map_err(|e| AudioError::PlayFailed(format!("{e}")))?;
        Ok(self.instances.insert(handle))
    }

    fn stop(&mut self, instance: InstanceHandle) {
        if let Some(handle) = self.instances.get_mut(instance) {
            handle.stop(Tween::default());
        }
    }

    fn instance_count(&self) -> usize {
        self.instances.len()
    }

    fn contains_instance(&self, handle: InstanceHandle) -> bool {
        self.instances.contains_key(handle)
    }

    fn set_instance_volume(&mut self, instance: InstanceHandle, volume: f32) {
        if let Some(handle) = self.instances.get_mut(instance) {
            handle.set_volume(volume as f64, Tween::default());
        }
    }

    fn set_master_volume(&mut self, volume: f32) {
        // Kira routes audio through a hierarchy of mixer tracks; the
        // main track represents the master bus. PR-1 keeps it simple
        // by adjusting the main track's volume directly.
        self.manager
            .main_track()
            .set_volume(volume as f64, Tween::default());
    }

    fn set_listener(&mut self, position: Vec3, rotation: Quat) {
        // Stored for spatial-audio follow-up (#64). Kira's spatial API
        // (`SpatialTrack`) lands when we wire the spatial sound system.
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

    /// Minimal valid wav header + PCM frame. Kira's symphonia decoder
    /// accepts wav out of the box (default feature flag).
    fn minimal_wav_bytes() -> Vec<u8> {
        // Build a 16-bit mono PCM wav with two zero samples — enough
        // for symphonia to decode without errors.
        let sample_rate: u32 = 44100;
        let bits_per_sample: u16 = 16;
        let num_channels: u16 = 1;
        let samples: [i16; 2] = [0, 0];

        let byte_rate = sample_rate * (bits_per_sample as u32 / 8) * num_channels as u32;
        let block_align = (bits_per_sample / 8) * num_channels;
        let data_size = (samples.len() as u32) * (bits_per_sample as u32 / 8);
        let chunk_size = 36 + data_size;

        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&chunk_size.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&num_channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits_per_sample.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn mock_backend_constructs() {
        let backend = KiraBackend::new_with_mock_backend()
            .expect("mock backend should always succeed");
        assert_eq!(backend.sound_count(), 0);
        assert_eq!(backend.instance_count(), 0);
    }

    #[test]
    fn load_minimal_wav_succeeds() {
        let mut backend = KiraBackend::new_with_mock_backend().unwrap();
        let bytes = minimal_wav_bytes();
        let handle = backend.load_sound(&bytes).expect("wav decode");
        assert!(backend.contains_sound(handle));
        assert_eq!(backend.sound_count(), 1);
    }

    #[test]
    fn invalid_bytes_return_decode_error() {
        let mut backend = KiraBackend::new_with_mock_backend().unwrap();
        let err = backend.load_sound(b"not a real audio").unwrap_err();
        assert!(matches!(err, AudioError::Decode(_)));
    }

    #[test]
    fn play_with_stale_handle_errs() {
        let mut backend = KiraBackend::new_with_mock_backend().unwrap();
        let h = backend.load_sound(&minimal_wav_bytes()).unwrap();
        backend.unload_sound(h);
        let err = backend.play(h, PlayParams::default()).unwrap_err();
        assert!(matches!(err, AudioError::SoundNotFound));
    }

    #[test]
    fn play_returns_live_instance() {
        let mut backend = KiraBackend::new_with_mock_backend().unwrap();
        let h = backend.load_sound(&minimal_wav_bytes()).unwrap();
        let inst = backend.play(h, PlayParams::default()).unwrap();
        assert!(backend.contains_instance(inst));
        assert_eq!(backend.instance_count(), 1);
    }

    #[test]
    fn listener_round_trips() {
        let mut backend = KiraBackend::new_with_mock_backend().unwrap();
        let pos = Vec3::new(5.0, 0.0, -3.0);
        let rot = Quat::from_rotation_x(0.5);
        backend.set_listener(pos, rot);
        let (got_pos, got_rot) = backend.listener();
        assert_eq!(got_pos, pos);
        assert!((got_rot.dot(rot)).abs() > 0.999);
    }
}
