//! Videos in the UI (#1793): each plays here, in the renderer, and its frame is drawn on the GPU into
//! its slot of the page atlas, so a frame never crosses the CPU or the remote wire.

use std::sync::Arc;
use std::time::{Duration, Instant};

use kooch_ecs::UiDrawList;
use kooch_video::Player;

use crate::video::{VideoConverter, VideoPlanes, VideoTarget};

/// A video: its key, its file, the player while it is drawn, the planes of the frame it shows, and
/// the clock epoch it runs from, with when it last ran and how long it has run.
type Playing = (
    u64,
    Arc<[u8]>,
    Option<Player>,
    Option<VideoPlanes>,
    Option<(u32, Instant, Duration)>,
);

impl super::UiPass {
    /// Frees what the videos hold while this pass is not drawn; they start over when it is.
    pub fn park_videos(&mut self) {
        self.videos.park();
    }

    /// The videos with no frame since they started or sought, for the host to hold their clocks.
    pub fn video_waits(&self) -> kooch_ecs::UiVideoWaits {
        kooch_ecs::UiVideoWaits(self.videos.waits())
    }
}

#[derive(Default)]
pub(super) struct UiVideos {
    converter: Option<VideoConverter>,
    playing: Vec<Playing>,
}

impl UiVideos {
    /// Starts new videos, stops hidden ones and draws each due frame into `pages`. `rewritten`: the
    /// pages were replaced or uploaded to this frame, so every slot is drawn again.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        list: &mut UiDrawList,
        pages: &wgpu::Texture,
        rewritten: bool,
    ) {
        for source in std::mem::take(&mut list.video_sources) {
            if !self.playing.iter().any(|(key, ..)| *key == source.key) {
                self.playing
                    .push((source.key, source.data, None, None, None));
            }
        }
        let shown = &list.videos;
        self.playing
            .retain(|(key, ..)| shown.iter().any(|video| video.key == *key));
        if self.playing.is_empty() {
            return;
        }
        let converter = self
            .converter
            .get_or_insert_with(|| VideoConverter::new(device));
        for video in shown {
            let Some((_, bytes, player, planes, since)) =
                self.playing.iter_mut().find(|(key, ..)| *key == video.key)
            else {
                continue;
            };
            // The host's clock, run on from when its epoch reached this renderer; held, like the
            // host's, while the player waits for a frame.
            let clock = video.clock;
            let now = Instant::now();
            let (_, last, ran) = match since {
                Some(run) if run.0 == clock.epoch => run,
                _ => since.insert((clock.epoch, now, Duration::ZERO)),
            };
            let player = player.get_or_insert_with(|| Player::start(bytes.clone()));
            if !player.waiting() {
                *ran += now - *last;
            }
            *last = now;
            let due = player.frame_at(clock.at(*ran, player.duration()));
            if let Some(picture) = &due {
                converter.upload(device, queue, planes, picture);
            }
            let Some(planes) = planes.as_ref() else {
                continue;
            };
            if due.is_none() && !rewritten {
                continue;
            }
            let target = VideoTarget {
                texture: pages,
                origin: [video.rect[0], video.rect[1]],
                border: true,
                size: Some([
                    video.rect[2].saturating_sub(video.rect[0] + 2),
                    video.rect[3].saturating_sub(video.rect[1] + 2),
                ]),
            };
            converter.draw(device, queue, encoder, planes, target);
        }
    }

    /// The keys of the videos waiting for a frame, for the host to hold their clocks.
    pub(super) fn waits(&self) -> Vec<u64> {
        let waiting = |(_, _, player, ..): &&Playing| player.as_ref().is_some_and(Player::waiting);
        self.playing
            .iter()
            .filter(waiting)
            .map(|(key, ..)| *key)
            .collect()
    }

    /// Stops every video until it is drawn again, from its first frame.
    /// 🔴 A 4K decoder holds 18 pictures, 215 MB, and a player nobody draws keeps them (#1894).
    pub(super) fn park(&mut self) {
        // The run stays: the host's clock ran on while nothing drew it.
        for (_, _, player, planes, _) in &mut self.playing {
            *player = None;
            *planes = None;
        }
    }
}
