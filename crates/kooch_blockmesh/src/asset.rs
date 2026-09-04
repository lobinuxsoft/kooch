//! Loading a `BlockMesh` from its `.blockmesh.ron` sidecar.

use std::fmt;

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use crate::BlockMesh;

/// What a block mesh file is called. Doubled up like `Material`'s, so a
/// `.ron` in an asset folder says which of several RON types it is
/// before anything opens it.
pub const BLOCK_MESH_EXTENSION: &str = "blockmesh.ron";

/// Reads `BlockMesh` assets from RON, the same authoring format
/// `Material` uses — a level's geometry is diffable text, and a corner
/// that moved shows up as a line that changed.
#[derive(Debug, Default, Clone, Copy)]
pub struct BlockMeshLoader;

impl AssetLoader<BlockMesh> for BlockMeshLoader {
    fn extensions(&self) -> &[&'static str] {
        &["ron"]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<BlockMesh> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(BlockMeshParseError::Utf8(e))))?;
        ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(BlockMeshParseError::Ron(e))))
    }
}

/// What can go wrong reading one.
#[derive(Debug)]
pub enum BlockMeshParseError {
    Utf8(std::str::Utf8Error),
    Ron(ron::error::SpannedError),
}

impl fmt::Display for BlockMeshParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "block mesh RON is not valid UTF-8: {e}"),
            Self::Ron(e) => write!(f, "block mesh RON parse failed: {e}"),
        }
    }
}

impl std::error::Error for BlockMeshParseError {}

// Declared beside the type, so any binary linking this crate gets both
// the loader and `Assets<BlockMesh>` without listing it anywhere.
kooch_core::register_asset!(BlockMesh, BlockMeshLoader);
