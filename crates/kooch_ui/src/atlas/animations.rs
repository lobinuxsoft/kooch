//! Animated images and videos (#1793). A GIF's frame due is rewritten into its slot in place, so its
//! quads never change and only that region is uploaded; a video is only named, and the renderer
//! draws its frames into the slot itself.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use kooch_core::Guid;
use kooch_core::gif::GifFrames;
use kooch_ecs::{UiVideo, UiVideoSource};

use super::GlyphAtlas;

struct Animation {
    frames: Arc<GifFrames>,
    start: Instant,
    shown: usize,
}

/// Each animated image and video the atlas holds, by asset.
#[derive(Default)]
pub(crate) struct Animations {
    gifs: HashMap<Guid, Animation>,
    /// Each video's key and file.
    videos: HashMap<Guid, (u64, Arc<[u8]>)>,
    /// The videos last shown, and the keys the renderer was handed: a hidden one is stopped there,
    /// so it is handed again when it shows.
    shown: Vec<UiVideo>,
    sent: Vec<u64>,
}

impl GlyphAtlas {
    /// Plays `frames` in `guid`'s slot from its first frame; `None` stops it.
    pub(crate) fn set_animation(&mut self, guid: Guid, frames: Option<Arc<GifFrames>>) {
        match frames {
            Some(frames) => self.animations.gifs.insert(
                guid,
                Animation {
                    frames,
                    start: Instant::now(),
                    shown: 0,
                },
            ),
            None => self.animations.gifs.remove(&guid),
        };
    }

    /// Plays `video`, a key and its WebM, in `guid`'s slot; `None` stops it.
    pub(crate) fn set_video(&mut self, guid: Guid, video: Option<(u64, Arc<[u8]>)>) {
        match video {
            Some(video) => self.animations.videos.insert(guid, video),
            None => self.animations.videos.remove(&guid),
        };
    }

    /// The videos shown, and the files of those the renderer has not been handed yet.
    pub(crate) fn take_videos(&mut self) -> (Vec<UiVideo>, Vec<UiVideoSource>) {
        let animations = &mut self.animations;
        let sources = animations
            .shown
            .iter()
            .filter(|video| !animations.sent.contains(&video.key))
            .filter_map(|video| {
                let (_, data) = animations
                    .videos
                    .values()
                    .find(|(key, _)| *key == video.key)?;
                Some(UiVideoSource {
                    key: video.key,
                    data: data.clone(),
                })
            })
            .collect();
        animations.sent = animations.shown.iter().map(|video| video.key).collect();
        (animations.shown.clone(), sources)
    }

    /// Writes the frame due into each of `shown`'s slots whose frame changed.
    pub(crate) fn advance(&mut self, shown: impl Iterator<Item = Guid>) {
        // Lifted out so a frame can be written without copying it.
        let mut animations = std::mem::take(&mut self.animations);
        animations.shown.clear();
        for guid in shown {
            if let Some(&(key, _)) = animations.videos.get(&guid) {
                // The slot with its border: the renderer repeats the edge there too.
                if let Some(&(_, image)) = self.images.get(&(guid, 0)) {
                    let [x0, y0, x1, y1] = image.rect;
                    animations.shown.push(UiVideo {
                        key,
                        rect: [x0 - 1, y0 - 1, x1 + 1, y1 + 1],
                    });
                }
                continue;
            }
            let Some(animation) = animations.gifs.get_mut(&guid) else {
                continue;
            };
            let due = animation.frames.due(animation.start.elapsed());
            if due == animation.shown {
                continue;
            }
            // A cleared atlas has no slot until the image packs again, from its first frame.
            let Some(&(_, image)) = self.images.get(&(guid, 0)) else {
                continue;
            };
            animation.shown = due;
            if let Some(frame) = animation.frames.frames.get(due) {
                let corner = [image.rect[0] - 1, image.rect[1] - 1];
                self.blit(corner, animation.frames.size, frame);
            }
        }
        self.animations = animations;
    }
}
