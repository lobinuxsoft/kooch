//! A video's frames decoded a few ahead on their own thread, asked for by time: the caller owns the
//! clock, so it can pause, seek and loop it, and the player follows.

use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::stream::{Picture, Stream};

/// Frames decoded ahead of the clock: enough to ride out a slow one, few at 3 MB a 1080p frame.
const AHEAD: usize = 2;
/// Further ahead of the newest frame than this is a jump: decoding through it would lag behind.
const JUMP: Duration = Duration::from_secs(1);

/// How far behind the frame shown the clock may land without rewinding.
const SETTLE: Duration = Duration::from_millis(250);

/// A seek: its generation, so frames decoded before it are dropped, and where it lands.
type Seek = (u32, Duration);

pub struct Player {
    /// Behind a lock only so a player can sit in a `Sync` resource; reached through `&mut`.
    frames: Mutex<Receiver<(u32, Picture)>>,
    seeks: Mutex<Sender<Seek>>,
    generation: u32,
    /// A frame received and not due yet.
    pending: Option<Picture>,
    /// When the frame last handed out shows.
    shown: Option<Duration>,
    /// Where the last seek lands: until its first frame arrives, what is "decoded so far".
    sought: Option<Duration>,
    duration: Option<Duration>,
    /// The decoding thread is gone: nothing more will arrive.
    stopped: bool,
    /// For [`Player::due`], which keeps its own clock.
    start: Instant,
}

impl Player {
    /// Starts `bytes`, a WebM, from its first frame.
    pub fn start(bytes: Arc<[u8]>) -> Self {
        let duration = crate::duration(&bytes);
        let (send, frames) = sync_channel(AHEAD);
        let (seek, seeks) = channel();
        let spawned = std::thread::Builder::new()
            .name("kooch-video".into())
            .spawn(move || play(bytes, send, seeks));
        if let Err(error) = spawned {
            tracing::warn!("video not played: failed to start its thread: {error}");
        }
        Self {
            frames: Mutex::new(frames),
            seeks: Mutex::new(seek),
            generation: 0,
            pending: None,
            shown: None,
            sought: None,
            duration,
            stopped: false,
            start: Instant::now(),
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Whether no frame has shown since the start or the last seek, and one is still coming.
    pub fn waiting(&self) -> bool {
        self.shown.is_none() && !self.stopped
    }

    /// The frame due on the player's own clock, looping: for a video nothing controls.
    pub fn due(&mut self) -> Option<Picture> {
        let elapsed = self.start.elapsed();
        let at = match self.duration {
            Some(length) if !length.is_zero() => {
                Duration::from_nanos((elapsed.as_nanos() % length.as_nanos()) as u64)
            }
            _ => elapsed,
        };
        self.frame_at(at)
    }

    /// The newest frame due at `at`, the video's own time, when it differs from the last one. A
    /// time behind that frame, or far past what is decoded, seeks there.
    pub fn frame_at(&mut self, at: Duration) -> Option<Picture> {
        // 🔴 Without `sought`, every frame before a seek's first one read as another jump past
        // 0 + JUMP, and re-seeking each frame never let the decoder deliver one.
        let newest = self
            .pending
            .as_ref()
            .map(|p| p.at)
            .or(self.shown)
            .or(self.sought);
        // A clock re-anchored a little behind the frame shown is not a seek back: rewinding to the
        // keyframe for it can cost seconds of decoding.
        // 🔴 Measured from where a pending seek lands too: without it, a seek back before the last
        // one's first frame was never sent, and the player waited on frames that never came due.
        let behind = self
            .shown
            .or(self.sought)
            .is_some_and(|shown| at + SETTLE < shown);
        if behind || at > newest.unwrap_or_default() + JUMP {
            self.seek(at);
        }
        let frames = self
            .frames
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let mut shown = None;
        loop {
            let next = match self.pending.take() {
                Some(picture) => picture,
                None => match frames.try_recv() {
                    Ok((generation, _)) if generation != self.generation => continue,
                    Ok((_, picture)) => picture,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.stopped = true;
                        break;
                    }
                },
            };
            if next.at > at {
                self.pending = Some(next);
                break;
            }
            shown = Some(next);
        }
        if let Some(picture) = &shown {
            self.shown = Some(picture.at);
        }
        shown
    }

    fn seek(&mut self, at: Duration) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.shown = None;
        self.sought = Some(at);
        let seeks = self.seeks.get_mut().unwrap_or_else(PoisonError::into_inner);
        if seeks.send((self.generation, at)).is_err() {
            return;
        }
        // Frees the decoder if it is waiting to hand over a frame from before the seek.
        let frames = self
            .frames
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        while frames.try_recv().is_ok() {}
    }
}

/// The decoding thread's place: where a seek lands, and the frame that shows there, held until
/// the next one passes it.
struct Decoding {
    stream: Stream<Arc<[u8]>>,
    bytes: Arc<[u8]>,
    keyframes: Option<Vec<u64>>,
    generation: u32,
    target: Option<(Duration, Option<Picture>)>,
}

impl Decoding {
    fn land(&mut self, (generation, at): Seek) -> Result<(), String> {
        let (stream, bytes) = (&self.stream, &self.bytes);
        let index = self
            .keyframes
            .get_or_insert_with(|| stream.keyframes(bytes.clone()));
        self.stream.seek(at, index)?;
        self.generation = generation;
        self.target = Some((at, None));
        Ok(())
    }
}

/// Decodes in order, landing on each seek it is asked for; ends with its [`Player`].
fn play(bytes: Arc<[u8]>, send: SyncSender<(u32, Picture)>, seeks: Receiver<Seek>) {
    let stream = match Stream::open(bytes.clone()) {
        Ok(stream) => stream,
        Err(error) => return tracing::warn!("video not played: {error}"),
    };
    let mut at = Decoding {
        stream,
        bytes,
        keyframes: None,
        generation: 0,
        target: None,
    };
    loop {
        if let Some(seek) = seeks.try_iter().last()
            && let Err(error) = at.land(seek)
        {
            return tracing::warn!("video stopped: {error}");
        }
        let generation = at.generation;
        match at.stream.next() {
            Ok(Some(picture)) => {
                if let Some((target, held)) = &mut at.target {
                    if picture.at <= *target {
                        *held = Some(picture);
                        continue;
                    }
                    if let Some(held) = held.take()
                        && send.send((generation, held)).is_err()
                    {
                        return;
                    }
                    at.target = None;
                }
                if send.send((generation, picture)).is_err() {
                    return;
                }
            }
            Ok(None) => {
                if let Some((_, Some(held))) = at.target.take()
                    && send.send((generation, held)).is_err()
                {
                    return;
                }
                // Past the last frame: nothing to decode until a seek, a loop's included.
                let Ok(seek) = seeks.recv() else { return };
                if let Err(error) = at.land(seek) {
                    return tracing::warn!("video stopped: {error}");
                }
            }
            Err(error) => return tracing::warn!("video stopped: {error}"),
        }
    }
}
