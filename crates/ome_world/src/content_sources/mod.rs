//! Default [`crate::content::ChunkContentSource`] implementations.
//!
//! Each submodule ships one strategy. They all live behind the same
//! `Box<dyn ChunkContentSource>` boundary on `ChunkManager`, so the
//! runtime swaps between them without touching the streaming layer.

pub mod city;

pub use city::ProceduralCitySource;
