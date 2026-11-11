//! Video (#1793): a WebM's AV1 track decoded by dav1d on its own thread, handed over as YUV planes
//! for the GPU to convert where the frame lands.

mod av1;
mod player;
mod stream;

use std::path::Path;

pub use av1::Plane;
pub use player::Player;
pub use stream::Picture;

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("webm"))
}

/// The first frame of `bytes`, a WebM: what shows before the player has one.
pub fn first_frame(bytes: &[u8]) -> Result<Picture, String> {
    stream::Stream::open(bytes)?
        .next()?
        .ok_or_else(|| "the video has no frames".into())
}
