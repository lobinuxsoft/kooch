//! Controlling the video a node shows (#1793): its clock lives here, where the game runs, and the
//! renderer follows it. From code through [`UiTree`], from a file through `video.*` actions.

use std::time::Duration;

use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::{Entity, UiVideoClock, UiVideoWaits};

use crate::{GlyphAtlas, UiEvent, UiEventKind, UiNode, UiPointer, UiTree, UiTrees};

/// Where a video is and how it plays, as the game sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoStatus {
    pub position: Duration,
    /// `None` until the file is loaded, or when its header gives none.
    pub length: Option<Duration>,
    pub playing: bool,
    pub looping: bool,
    pub rate: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct VideoNode {
    node: UiNode,
    status: VideoStatus,
    /// What the renderer was last told; a change starts a new epoch.
    clock: UiVideoClock,
    changed: bool,
}

/// A control showing a video's place: a slider that seeks it, or a label with its times.
#[derive(Debug, Clone)]
pub(crate) struct VideoBind {
    node: UiNode,
    target: Box<str>,
    seeks: bool,
    /// The slider value or the whole seconds last written: a different one is the user's.
    shown: f32,
}

/// Attribute keys a video node or a control reads.
pub(crate) const VIDEO_KEYS: [&str; 5] = [
    "autoplay",
    "loop",
    "on:end",
    "bind:video",
    "bind:video-time",
];

impl UiTree {
    /// Where `node`'s video is; `None` when it shows none.
    pub fn video(&self, node: UiNode) -> Option<VideoStatus> {
        self.videos
            .iter()
            .find(|v| v.node == node)
            .map(|v| v.status)
    }

    pub fn play_video(&mut self, node: UiNode) {
        self.edit_video(node, |status| status.playing = true);
    }

    pub fn pause_video(&mut self, node: UiNode) {
        self.edit_video(node, |status| status.playing = false);
    }

    /// Pauses and goes back to the first frame.
    pub fn stop_video(&mut self, node: UiNode) {
        self.edit_video(node, |status| {
            status.playing = false;
            status.position = Duration::ZERO;
        });
    }

    pub fn seek_video(&mut self, node: UiNode, at: Duration) {
        self.edit_video(node, |status| {
            status.position = status.length.map_or(at, |length| at.min(length));
        });
    }

    /// Moves by `seconds`, back when negative.
    pub fn skip_video(&mut self, node: UiNode, seconds: f32) {
        let Some(status) = self.video(node) else {
            return;
        };
        let at = (status.position.as_secs_f32() + seconds).max(0.0);
        self.seek_video(node, Duration::from_secs_f32(at));
    }

    pub fn set_video_loop(&mut self, node: UiNode, looping: bool) {
        self.edit_video(node, |status| status.looping = looping);
    }

    /// 1 is normal speed; 0 or less pauses it.
    pub fn set_video_rate(&mut self, node: UiNode, rate: f32) {
        self.edit_video(node, |status| status.rate = rate.max(0.0));
    }

    fn edit_video(&mut self, node: UiNode, edit: impl FnOnce(&mut VideoStatus)) {
        let entry = self.video_entry(node);
        edit(&mut entry.status);
        entry.changed = true;
    }

    fn video_entry(&mut self, node: UiNode) -> &mut VideoNode {
        let index = match self.videos.iter().position(|v| v.node == node) {
            Some(index) => index,
            None => {
                self.videos.push(VideoNode {
                    node,
                    status: VideoStatus {
                        position: Duration::ZERO,
                        length: None,
                        playing: true,
                        looping: true,
                        rate: 1.0,
                    },
                    clock: UiVideoClock::default(),
                    changed: true,
                });
                self.videos.len() - 1
            }
        };
        &mut self.videos[index]
    }

    /// A video attribute from the file: `autoplay`, `loop`, `on:end`, `bind:video(-time)`.
    pub(crate) fn video_attribute(&mut self, node: UiNode, key: &str, value: &str) {
        let on = value != "false";
        match key {
            "autoplay" => self.video_entry(node).status.playing = on,
            "loop" => self.video_entry(node).status.looping = on,
            "on:end" => self.handlers.push((node, UiEventKind::End, value.into())),
            "bind:video" | "bind:video-time" => self.video_binds.push(VideoBind {
                node,
                target: value.into(),
                seeks: key == "bind:video",
                shown: f32::NAN,
            }),
            _ => {}
        }
    }
}

/// Runs each document's video clocks on by this frame, then its controls; after gameplay's
/// `Update`, which is where a game plays and pauses them.
pub fn play_videos(resources: &mut Resources) {
    let dt = Time::of(resources).unscaled_delta();
    let waits = resources.get::<UiVideoWaits>().cloned().unwrap_or_default();
    let held = resources.get::<UiPointer>().and_then(UiPointer::held);
    let Some(mut trees) = resources.get_mut::<UiTrees>().map(std::mem::take) else {
        return;
    };
    let mut ended = Vec::new();
    if let Some(atlas) = resources.get_mut::<GlyphAtlas>() {
        trees.for_each_entity_mut(|document, tree| {
            run_clocks(document, tree, atlas, (dt, &waits), &mut ended);
            run_binds(
                tree,
                held.filter(|(entity, _)| *entity == document)
                    .map(|(_, node)| node),
            );
        });
    }
    if let Some(slot) = resources.get_mut::<UiTrees>() {
        *slot = trees;
    }
    for event in ended {
        crate::pointer::dispatch(resources, event);
    }
}

fn run_clocks(
    document: Entity,
    tree: &mut UiTree,
    atlas: &mut GlyphAtlas,
    (dt, waits): (Duration, &UiVideoWaits),
    ended: &mut Vec<UiEvent>,
) {
    // Every node showing a video has a clock, controlled or not.
    for index in 0..tree.image.len() {
        let shows = tree.image[index].is_some_and(|image| atlas.is_video(image.guid));
        if shows {
            tree.video_entry(UiNode(index as u32));
        }
    }
    for index in 0..tree.videos.len() {
        let node = tree.videos[index].node;
        let Some(guid) = tree
            .image
            .get(node.0 as usize)
            .copied()
            .flatten()
            .map(|i| i.guid)
        else {
            continue;
        };
        if !atlas.is_video(guid) {
            continue;
        }
        let on_end: Option<String> = tree.handler(node, UiEventKind::End).map(Into::into);
        // The renderer has no frame for it yet: running on would leave the picture behind.
        let waiting = atlas
            .video_key(guid)
            .is_some_and(|key| waits.0.contains(&key));
        let video = &mut tree.videos[index];
        let status = &mut video.status;
        if status.length.is_none() {
            status.length = atlas
                .video_bytes(guid)
                .and_then(|b| kooch_video::duration(b));
        }
        if status.playing && !waiting {
            status.position += dt.mul_f32(status.rate);
        }
        if let Some(length) = status.length
            && status.position >= length
        {
            if status.looping && !length.is_zero() {
                let wrapped = status.position.as_nanos() % length.as_nanos();
                status.position = Duration::from_nanos(wrapped as u64);
            } else if status.playing {
                status.position = length;
                status.playing = false;
                video.changed = true;
                if let Some(action) = on_end {
                    ended.push(UiEvent {
                        document,
                        node,
                        kind: UiEventKind::End,
                        action,
                        value: None,
                    });
                }
            }
        }
        if std::mem::take(&mut video.changed) {
            let status = video.status;
            video.clock = UiVideoClock {
                at_us: status.position.as_micros() as u64,
                epoch: video.clock.epoch.wrapping_add(1),
                rate: if status.playing { status.rate } else { 0.0 },
                looping: status.looping,
            };
        }
        atlas.set_clock(guid, video.clock);
    }
}

/// `held`: the node being dragged, whose slider seeks once, on release.
fn run_binds(tree: &mut UiTree, held: Option<UiNode>) {
    for index in 0..tree.video_binds.len() {
        let bind = tree.video_binds[index].clone();
        let Some(target) = tree.query(&bind.target) else {
            continue;
        };
        let Some(status) = tree.video(target) else {
            continue;
        };
        let length = status.length.unwrap_or_default().as_secs_f32();
        let position = status.position.as_secs_f32();
        // 🔴 Seeking at every step of a drag restarted the decode each frame: back from a far
        // keyframe it never showed one (#1793).
        if bind.seeks && held == Some(bind.node) {
            continue;
        }
        let shown = if bind.seeks {
            let Some((min, max)) = tree.control(bind.node).map(|c| (c.min, c.max)) else {
                continue;
            };
            let value = tree.value(bind.node).unwrap_or(min);
            if !bind.shown.is_nan() && (value - bind.shown).abs() > f32::EPSILON && max > min {
                let fraction = ((value - min) / (max - min)).clamp(0.0, 1.0);
                tree.seek_video(target, Duration::from_secs_f32(fraction * length));
                value
            } else {
                let fraction = if length > 0.0 { position / length } else { 0.0 };
                tree.set_value(bind.node, min + fraction * (max - min));
                // 🔴 As the slider stored it: its step rounds, and a rounded value read back next
                // frame looked like a drag and sought again (#1793).
                tree.value(bind.node).unwrap_or(min)
            }
        } else {
            let whole = position.floor();
            if whole != bind.shown {
                tree.write_text(
                    bind.node,
                    &format!("{} / {}", clock(position), clock(length)),
                );
            }
            whole
        };
        tree.video_binds[index].shown = shown;
    }
}

/// `m:ss`.
fn clock(seconds: f32) -> String {
    let whole = seconds.max(0.0) as u32;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// `video.<verb> <node> [amount]`, run on the document that sent it; `false` for other actions.
pub(crate) fn run_action(resources: &mut Resources, event: &UiEvent) -> bool {
    let mut words = event.action.split_whitespace();
    let Some(verb) = words.next().and_then(|w| w.strip_prefix("video.")) else {
        return false;
    };
    let (Some(name), amount) = (
        words.next(),
        words.next().and_then(|w| w.parse::<f32>().ok()),
    ) else {
        tracing::warn!("UI action `{}` names no video node", event.action);
        return true;
    };
    let Some(tree) = resources
        .get_mut::<UiTrees>()
        .and_then(|t| t.get_mut(event.document))
    else {
        return true;
    };
    let Some(node) = tree.query(name) else {
        tracing::warn!("UI action `{}`: no node named `{name}`", event.action);
        return true;
    };
    let playing = tree.video(node).is_some_and(|status| status.playing);
    match verb {
        "play" => tree.play_video(node),
        "pause" => tree.pause_video(node),
        "toggle" if playing => tree.pause_video(node),
        "toggle" => tree.play_video(node),
        "stop" => tree.stop_video(node),
        "restart" => {
            tree.seek_video(node, Duration::ZERO);
            tree.play_video(node);
        }
        "skip" => tree.skip_video(node, amount.unwrap_or(10.0)),
        "seek" => tree.seek_video(
            node,
            Duration::from_secs_f32(amount.unwrap_or(0.0).max(0.0)),
        ),
        "loop" => {
            let looping = tree.video(node).is_some_and(|status| status.looping);
            tree.set_video_loop(node, !looping);
        }
        "rate" => tree.set_video_rate(node, amount.unwrap_or(1.0)),
        _ => tracing::warn!("UI action `{}`: unknown video verb `{verb}`", event.action),
    }
    true
}
