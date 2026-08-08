//! kooch_audio — audio subsystem.
//!
//! [`AudioBackend`] is the trait the engine consumes; concrete impls
//! ([`KiraBackend`] today, future SDL_mixer / WebAudio / oddio) plug
//! behind it. [`MockAudioBackend`] is the headless test backend.
//!
//! # Architecture
//!
//! - [`backend`] — trait + cross-backend types (SoundHandle,
//!   InstanceHandle, PlayParams, AudioError)
//! - [`kira_backend`] — concrete [`KiraBackend`] (Kira 0.9 with
//!   default mp3/ogg/flac/wav decoders)
//! - [`mock_backend`] — [`MockAudioBackend`] for tests + tooling
//!
//! # Out of scope (follow-ups)
//!
//! - Spatial audio (kira `SpatialTrack` integration) — #64
//! - Sound pooling / object pool for low-latency SFX — #66
//! - Mixer bus hierarchy (music vs SFX vs voice)
//! - Real-time effects (reverb, lowpass, etc.)
//! - Async / streaming sounds for long music tracks

pub mod backend;
pub mod kira_backend;
pub mod mock_backend;

pub use backend::{AudioBackend, AudioError, InstanceHandle, PlayParams, SoundHandle};
pub use kira_backend::KiraBackend;
pub use mock_backend::MockAudioBackend;
