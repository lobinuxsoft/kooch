//! Videos in the UI (#1793): each plays here, in the renderer, and its frame is drawn on the GPU into
//! its atlas slot, so a frame never crosses the CPU atlas or the remote wire.

use std::sync::Arc;

use kooch_ecs::UiDrawList;
use kooch_video::Player;

use crate::video::{VideoConverter, VideoPlanes, VideoTarget};

/// A video: its key, its file, the player while it is drawn and the planes of the frame it shows.
type Playing = (u64, Arc<[u8]>, Option<Player>, Option<VideoPlanes>);

#[derive(Default)]
pub(super) struct UiVideos {
    converter: Option<VideoConverter>,
    playing: Vec<Playing>,
}

impl UiVideos {
    /// Starts new videos, stops hidden ones and draws each due frame into `atlas`. `rewritten`: the
    /// atlas was replaced or uploaded to this frame, so every slot is drawn again.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        list: &mut UiDrawList,
        atlas: &wgpu::Texture,
        rewritten: bool,
    ) {
        for source in std::mem::take(&mut list.video_sources) {
            if !self.playing.iter().any(|(key, ..)| *key == source.key) {
                self.playing.push((source.key, source.data, None, None));
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
            let Some((_, bytes, player, planes)) =
                self.playing.iter_mut().find(|(key, ..)| *key == video.key)
            else {
                continue;
            };
            let due = player
                .get_or_insert_with(|| Player::start(bytes.clone()))
                .due();
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
                texture: atlas,
                origin: [video.rect[0], video.rect[1]],
                border: true,
            };
            converter.draw(device, queue, encoder, planes, target);
        }
    }

    /// Stops every video until it is drawn again, from its first frame.
    /// 🔴 A 4K decoder holds 18 pictures, 215 MB, and a player nobody draws keeps them (#1894).
    pub(super) fn park(&mut self) {
        for (_, _, player, planes) in &mut self.playing {
            *player = None;
            *planes = None;
        }
    }
}
