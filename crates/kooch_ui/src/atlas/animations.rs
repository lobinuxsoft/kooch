//! Animated images and videos (#1793): the frame due rewritten into the image's slot in place, so
//! its quads never change and only that region is uploaded.

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

/// Each animated image and video the atlas holds, by asset.
#[derive(Default)]
pub(crate) struct Animations {
    gifs: HashMap<Guid, Animation>,
    videos: HashMap<Guid, kooch_video::Player>,
    /// PROBE(#1793): frames blitted, their bytes and time, since the last log.
    probe: (u32, u64, std::time::Duration, Option<Instant>),
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

    /// Plays `player` in `guid`'s slot; `None` stops it.
    pub(crate) fn set_video(&mut self, guid: Guid, player: Option<kooch_video::Player>) {
        match player {
            Some(player) => self.animations.videos.insert(guid, player),
            None => self.animations.videos.remove(&guid),
        };
    }

    /// Writes the frame due into each of `shown`'s slots whose frame changed.
    pub(crate) fn advance(&mut self, shown: impl Iterator<Item = Guid>) {
        // Lifted out so a frame can be written without copying it.
        let mut animations = std::mem::take(&mut self.animations);
        for guid in shown {
            if let Some(player) = animations.videos.get_mut(&guid) {
                let (Some(&(_, image)), Some(picture)) =
                    (self.images.get(&(guid, 0)), player.due())
                else {
                    continue;
                };
                let corner = [image.rect[0] - 1, image.rect[1] - 1];
                let began = Instant::now();
                self.blit(corner, picture.size, &picture.rgba);
                let probe = &mut animations.probe;
                probe.0 += 1;
                probe.1 += picture.rgba.len() as u64;
                probe.2 += began.elapsed();
                let since = *probe.3.get_or_insert_with(Instant::now);
                if since.elapsed().as_secs() >= 2 {
                    tracing::info!(
                        target: "video_probe",
                        %guid,
                        size = ?picture.size,
                        blits = probe.0,
                        mb = probe.1 / 1_000_000,
                        blit_ms = probe.2.as_millis() as u64,
                        skipped = player.skipped,
                        "atlas (2 s)"
                    );
                    *probe = (0, 0, std::time::Duration::ZERO, Some(Instant::now()));
                    player.skipped = 0;
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
