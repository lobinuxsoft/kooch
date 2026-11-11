//! A video playing in a loop: frames decoded a few ahead on their own thread, taken when due.

use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::stream::{Picture, Stream};

/// Frames decoded ahead of the clock: enough to ride out a slow one, few at 8 MB a 1080p frame.
const AHEAD: usize = 2;
/// The gap after the last frame when a video has a single one.
const STEP: Duration = Duration::from_millis(33);

pub struct Player {
    /// Behind a lock only so a player can sit in a `Sync` resource; reached through `&mut`.
    frames: Mutex<Receiver<Picture>>,
    start: Instant,
    /// A frame received and not due yet.
    pending: Option<Picture>,
}

impl Player {
    /// Starts `bytes`, a WebM, from its first frame.
    pub fn start(bytes: Arc<[u8]>) -> Self {
        let (send, frames) = sync_channel(AHEAD);
        let spawned = std::thread::Builder::new()
            .name("kooch-video".into())
            .spawn(move || play(bytes, send));
        if let Err(error) = spawned {
            tracing::warn!("video not played: failed to start its thread: {error}");
        }
        Self {
            frames: Mutex::new(frames),
            start: Instant::now(),
            pending: None,
        }
    }

    /// The newest frame due by now, when one came due since the last call.
    pub fn due(&mut self) -> Option<Picture> {
        let now = self.start.elapsed();
        let frames = self
            .frames
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let mut shown = None;
        loop {
            let next = match self.pending.take() {
                Some(picture) => picture,
                None => match frames.try_recv() {
                    Ok(picture) => picture,
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                },
            };
            if next.at > now {
                self.pending = Some(next);
                break;
            }
            shown = Some(next);
        }
        shown
    }
}

/// Decodes `bytes` again and again, each loop's times after the last; ends with its [`Player`].
fn play(bytes: Arc<[u8]>, send: SyncSender<Picture>) {
    let mut base = Duration::ZERO;
    loop {
        let mut stream = match Stream::open(bytes.clone()) {
            Ok(stream) => stream,
            Err(error) => return tracing::warn!("video not played: {error}"),
        };
        let (mut last, mut step) = (None, STEP);
        loop {
            match stream.next() {
                Ok(Some(mut picture)) => {
                    if let Some(last) = last {
                        step = picture
                            .at
                            .saturating_sub(last)
                            .max(Duration::from_millis(1));
                    }
                    last = Some(picture.at);
                    picture.at += base;
                    if send.send(picture).is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(error) => return tracing::warn!("video stopped: {error}"),
            }
        }
        let Some(last) = last else {
            return tracing::warn!("video not played: it has no frames");
        };
        base += last + step;
    }
}
