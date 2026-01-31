//! ome_audio - Audio system for oh_my_engine
//!
//! Provides audio playback via kira, spatial audio,
//! gravity-aware audio orientation, and sound pooling.
//!
//! Enable the `audio` feature to include kira support.

/// Placeholder for future implementation
pub fn init() {
    #[cfg(feature = "audio")]
    tracing::info!("ome_audio initialized with kira");

    #[cfg(not(feature = "audio"))]
    tracing::info!("ome_audio initialized (audio feature disabled)");
}
