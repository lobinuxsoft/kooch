//! ABI-stable types for the plugin API.
//!
//! Defines the primitive types used across the FFI boundary:
//! stage constants, entity packing, and callback signatures.

use std::ffi::c_void;

use crate::engine_api::EngineApi;

/// Stage constants matching `ome_core::Stage` `#[repr(u8)]` values.
///
/// Plugins use these with [`EngineApi::register_system`] to specify
/// when their systems should run.
pub mod stage {
    /// One-time initialization at application startup.
    pub const STARTUP: u8 = 0;
    /// Beginning of each frame.
    pub const FIRST: u8 = 1;
    /// Input event processing.
    pub const INPUT: u8 = 2;
    /// Preparation before main update.
    pub const PRE_UPDATE: u8 = 3;
    /// Main game logic.
    pub const UPDATE: u8 = 4;
    /// Cleanup after main update.
    pub const POST_UPDATE: u8 = 5;
    /// GPU synchronization.
    pub const GPU_SYNC: u8 = 6;
    /// GPU command submission.
    pub const GPU: u8 = 7;
    /// Physics simulation (fixed timestep).
    pub const PHYSICS: u8 = 8;
    /// Post-physics processing (fixed timestep).
    pub const POST_PHYSICS: u8 = 9;
    /// Preparation before rendering.
    pub const PRE_RENDER: u8 = 10;
    /// Main rendering.
    pub const RENDER: u8 = 11;
    /// Post-render cleanup.
    pub const POST_RENDER: u8 = 12;
    /// End of frame cleanup.
    pub const LAST: u8 = 13;
}

/// Callback invoked by the engine each frame for a registered system.
///
/// - `api`: pointer to a temporary [`EngineApi`] valid only for this invocation
/// - `userdata`: opaque pointer passed during registration
pub type SystemCallback = extern "C" fn(api: *mut EngineApi, userdata: *mut c_void);

/// Destructor for system callback userdata.
///
/// Called when the system is removed or the engine shuts down.
/// Must be safe to call exactly once.
pub type UserdataDrop = unsafe extern "C" fn(userdata: *mut c_void);

/// Packs entity index and generation into a single `u64` handle.
///
/// Layout: lower 32 bits = index, upper 32 bits = generation.
///
/// # Example
/// ```
/// use ome_plugin_api::types::{pack_entity, unpack_entity};
///
/// let handle = pack_entity(42, 7);
/// assert_eq!(unpack_entity(handle), (42, 7));
/// ```
#[inline]
pub const fn pack_entity(index: u32, generation: u32) -> u64 {
    (index as u64) | ((generation as u64) << 32)
}

/// Unpacks a `u64` entity handle into `(index, generation)`.
#[inline]
pub const fn unpack_entity(handle: u64) -> (u32, u32) {
    (handle as u32, (handle >> 32) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let (idx, generation) = (42, 7);
        let handle = pack_entity(idx, generation);
        assert_eq!(unpack_entity(handle), (idx, generation));
    }

    #[test]
    fn pack_unpack_zero() {
        assert_eq!(unpack_entity(pack_entity(0, 0)), (0, 0));
    }

    #[test]
    fn pack_unpack_max() {
        let handle = pack_entity(u32::MAX, u32::MAX);
        assert_eq!(unpack_entity(handle), (u32::MAX, u32::MAX));
    }

    #[test]
    fn pack_layout() {
        let handle = pack_entity(0xDEAD, 0xBEEF);
        // Lower 32 bits = index
        assert_eq!(handle & 0xFFFF_FFFF, 0xDEAD);
        // Upper 32 bits = generation
        assert_eq!(handle >> 32, 0xBEEF);
    }

    #[test]
    fn stage_values_sequential() {
        assert_eq!(stage::STARTUP, 0);
        assert_eq!(stage::LAST, 13);
        assert!(stage::UPDATE < stage::RENDER);
    }
}
