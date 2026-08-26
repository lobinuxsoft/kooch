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

use crate::backend::{AudioBackend, AudioError, InstanceHandle, PlayParams, SoundHandle};

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
        let data =
            StaticSoundData::from_cursor(cursor).map_err(|e| AudioError::Decode(format!("{e}")))?;
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
mod tests;
