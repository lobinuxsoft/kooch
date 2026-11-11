//! Video (#1793): a WebM's AV1 track decoded by dav1d and converted to RGBA8 on its own thread,
//! so the UI atlas and a material texture take a frame the way they take a GIF's.

mod av1;
mod player;
mod stream;

use std::path::Path;

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
