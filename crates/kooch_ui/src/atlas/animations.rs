//! Animated images (#1793): the frame due rewritten into the image's slot in place, so its quads
//! never change and only that region is uploaded.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use kooch_core::Guid;
use kooch_core::gif::GifFrames;

use super::GlyphAtlas;

struct Animation {
    frames: Arc<GifFrames>,
    start: Instant,
    shown: usize,
}

/// Each animated image the atlas holds, by asset.
#[derive(Default)]
pub(crate) struct Animations(HashMap<Guid, Animation>);

impl GlyphAtlas {
    /// Plays `frames` in `guid`'s slot from its first frame; `None` stops it.
    pub(crate) fn set_animation(&mut self, guid: Guid, frames: Option<Arc<GifFrames>>) {
        match frames {
            Some(frames) => self.animations.0.insert(
                guid,
                Animation {
                    frames,
                    start: Instant::now(),
                    shown: 0,
                },
            ),
            None => self.animations.0.remove(&guid),
        };
    }

    /// Writes the frame due into each of `shown`'s slots whose frame changed.
    pub(crate) fn advance(&mut self, shown: impl Iterator<Item = Guid>) {
        // Lifted out so a frame can be written without copying it.
        let mut animations = std::mem::take(&mut self.animations);
        for guid in shown {
            let Some(animation) = animations.0.get_mut(&guid) else {
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
