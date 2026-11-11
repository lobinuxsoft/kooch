//! Animated GIFs (#1793): every frame decoded once, RGBA8 at the image's size, and when each is
//! due. The UI rewrites its atlas slot and a material its texture; both play by wall clock.

use std::path::Path;
use std::time::Duration;

/// Below this a frame's delay is taken as [`DEFAULT_DELAY`], as browsers do: a GIF saying 0 or
/// 10 ms means "as fast as you like", not 100 fps.
const MIN_DELAY: u32 = 20;
const DEFAULT_DELAY: u32 = 100;

/// A GIF's frames, and when each ends from the start of a loop, in ms.
#[derive(Debug)]
pub struct GifFrames {
    pub frames: Vec<Vec<u8>>,
    pub ends: Vec<u32>,
    pub size: (u32, u32),
}

impl GifFrames {
    /// The frame due `elapsed` after the first was shown, looping.
    pub fn due(&self, elapsed: Duration) -> usize {
        let total = self.ends.last().copied().unwrap_or(0).max(1);
        let at = (elapsed.as_millis() % total as u128) as u32;
        self.ends.partition_point(|&end| end <= at)
    }
}

pub fn is_gif(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"))
}

/// The frames of a GIF, or `None` for one with a single frame: drawn as a still.
pub fn decode(bytes: &[u8]) -> Result<Option<GifFrames>, String> {
    use image::AnimationDecoder;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|e| e.to_string())?;
    let mut frames = Vec::new();
    let mut ends = Vec::new();
    let mut size = (0, 0);
    let mut end = 0u32;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(|e| e.to_string())?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay = match numer / denom.max(1) {
            delay if delay < MIN_DELAY => DEFAULT_DELAY,
            delay => delay,
        };
        let rgba = frame.into_buffer();
        size = rgba.dimensions();
        end = end.saturating_add(delay);
        ends.push(end);
        frames.push(rgba.into_raw());
    }
    Ok((frames.len() > 1).then_some(GifFrames { frames, ends, size }))
}
