//! [`AudioBackend`] trait + shared types.
//!
//! Game code stores `Box<dyn AudioBackend>` as a Resource and calls
//! the trait methods. Concrete backends ([`crate::KiraBackend`] today,
//! future SDL_mixer / WebAudio / oddio) plug behind it.

use glam::{Quat, Vec3};
use std::fmt;

slotmap::new_key_type! {
    /// Handle to a loaded sound asset (decoded PCM in memory).
    pub struct SoundHandle;
}

slotmap::new_key_type! {
    /// Handle to a *playing* sound instance. Distinct from
    /// [`SoundHandle`]: one sound can be played many times (footsteps,
    /// gunshots, etc.), each play producing a fresh [`InstanceHandle`].
    pub struct InstanceHandle;
}

/// Construction settings for [`AudioBackend::play`].
#[derive(Debug, Clone, Copy)]
pub struct PlayParams {
    /// Linear gain. `1.0` = unmodified, `0.0` = silent.
    pub volume: f32,
    /// Playback rate / pitch. `1.0` = original.
    pub pitch: f32,
    /// Whether the sound should loop indefinitely.
    pub looping: bool,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            volume: 1.0,
            pitch: 1.0,
            looping: false,
        }
    }
}

/// Errors surfaced by [`AudioBackend`] operations.
#[derive(Debug)]
pub enum AudioError {
    /// Underlying audio device / backend init failed.
    BackendInit(String),
    /// Decoder rejected the byte stream.
    Decode(String),
    /// Unrecognised sound handle.
    SoundNotFound,
    /// Unrecognised instance handle.
    InstanceNotFound,
    /// Backend rejected the play request (resource limit, etc.).
    PlayFailed(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::BackendInit(msg) => write!(f, "audio backend init failed: {msg}"),
            AudioError::Decode(msg) => write!(f, "audio decode failed: {msg}"),
            AudioError::SoundNotFound => write!(f, "sound handle is stale"),
            AudioError::InstanceNotFound => write!(f, "instance handle is stale"),
            AudioError::PlayFailed(msg) => write!(f, "audio play failed: {msg}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Engine-facing audio interface.
///
/// # Lifecycle
///
/// 1. Engine inserts a backend at startup
///    (`Box<dyn AudioBackend>` as a Resource).
/// 2. Asset loader calls [`load_sound`](Self::load_sound) with decoded
///    bytes, stashes the returned [`SoundHandle`].
/// 3. Game code calls [`play`](Self::play) → [`InstanceHandle`] which
///    can be stopped, volume-tweaked, etc.
/// 4. [`unload_sound`](Self::unload_sound) frees the asset (active
///    instances continue until they finish).
pub trait AudioBackend: Send + Sync + 'static {
    /// Loads decoded audio bytes (mp3 / ogg / flac / wav supported by
    /// the production backend; raw PCM for the mock). Returns a handle
    /// the caller can pass to [`play`](Self::play).
    fn load_sound(&mut self, bytes: &[u8]) -> Result<SoundHandle, AudioError>;

    /// Drops the decoded sound data. Live instances continue playing
    /// (kira owns the audio data once `play` is called).
    fn unload_sound(&mut self, handle: SoundHandle);

    /// Number of loaded sounds (not playing instances).
    fn sound_count(&self) -> usize;

    /// Whether the sound handle is live.
    fn contains_sound(&self, handle: SoundHandle) -> bool;

    /// Starts a new instance of `sound`. The returned handle controls
    /// the instance — pass it to [`stop`](Self::stop) /
    /// [`set_instance_volume`](Self::set_instance_volume).
    fn play(
        &mut self,
        sound: SoundHandle,
        params: PlayParams,
    ) -> Result<InstanceHandle, AudioError>;

    /// Stops a playing instance. Silent no-op for stale handles.
    fn stop(&mut self, instance: InstanceHandle);

    /// Number of currently tracked instances. Includes ones that may
    /// have finished playing — backends prune lazily on the next mut
    /// call. Useful as a sanity ceiling for tests.
    fn instance_count(&self) -> usize;

    /// Whether the instance handle is live in the backend's tracking.
    fn contains_instance(&self, handle: InstanceHandle) -> bool;

    /// Per-instance volume (0.0 = silent, 1.0 = unmodified).
    fn set_instance_volume(&mut self, instance: InstanceHandle, volume: f32);

    /// Master / global volume. Applies to every instance.
    fn set_master_volume(&mut self, volume: f32);

    /// Updates the listener pose. Used for spatial audio attenuation
    /// in follow-ups; currently stored for backend-side use without
    /// gameplay-visible effect.
    fn set_listener(&mut self, position: Vec3, rotation: Quat);

    /// Reads the listener pose (position + rotation).
    fn listener(&self) -> (Vec3, Quat);
}

#[cfg(test)]
mod tests {
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
}
