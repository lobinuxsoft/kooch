//! How hard the main loop should spin — and how to wake it once it stops.
//!
//! A windowed app used to schedule the next frame at the end of every
//! frame, unconditionally, so the loop fed itself forever. Vsync capped
//! it at the refresh rate, which is why it cost one core rather than
//! eight; a core spent redrawing an unchanged image is still a core spent
//! on nothing, and on a handheld it is battery (#656).
//!
//! Two pieces make idling possible:
//!
//! - [`FrameRequest`] — what *this* frame decided the next one needs.
//!   Systems raise it; the runner reads it once and resets it. Raising is
//!   monotonic within a frame: the most urgent request wins, so no system
//!   can talk another out of a repaint it asked for.
//! - [`FrameWaker`] — a handle any thread can hold to break the loop out
//!   of a sleep. The remote server's listener thread needs exactly this:
//!   it blocks on a reply that only the main loop can produce, so a
//!   sleeping main loop would deadlock the editor talking to it.
//!
//! An app that never touches either keeps the old behaviour — the runner
//! treats a missing [`FrameRequest`] as [`FramePace::Continuous`]. A game
//! is *supposed* to spin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the next frame needs, in order of urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FramePace {
    /// Nothing is animating. Sleep until an event arrives.
    #[default]
    Wait,
    /// Sleep, but no longer than this — something is on a timer.
    After(Duration),
    /// Redraw as fast as the presenter allows.
    Continuous,
}

impl FramePace {
    /// The more urgent of the two.
    ///
    /// Ordering is `Continuous` > `After(shorter)` > `After(longer)` >
    /// `Wait`. Not an `Ord` impl: `After` is urgency-descending in its
    /// payload, which would make a derived ordering lie.
    pub fn most_urgent(self, other: Self) -> Self {
        match (self, other) {
            (Self::Continuous, _) | (_, Self::Continuous) => Self::Continuous,
            (Self::After(a), Self::After(b)) => Self::After(a.min(b)),
            (Self::After(d), Self::Wait) | (Self::Wait, Self::After(d)) => Self::After(d),
            (Self::Wait, Self::Wait) => Self::Wait,
        }
    }

    /// Reads egui's `repaint_delay` as a pace.
    ///
    /// egui reports `ZERO` for "repaint now" and `Duration::MAX` for
    /// "nothing is animating, wake me on an event". Anything between is
    /// a deadline — a tooltip fading in, a spinner, a blinking cursor.
    pub fn from_repaint_delay(delay: Duration) -> Self {
        if delay.is_zero() {
            Self::Continuous
        } else if delay == Duration::MAX {
            Self::Wait
        } else {
            Self::After(delay)
        }
    }
}

/// The pace this frame is asking the next one to run at.
///
/// Insert it to opt an app into idling; leave it out to spin forever.
/// The baseline is what the accumulator resets to after each read, so an
/// app that wants to sleep by default sets `Wait` and every frame starts
/// from there.
#[derive(Debug)]
pub struct FrameRequest {
    baseline: FramePace,
    pending: FramePace,
}

impl Default for FrameRequest {
    fn default() -> Self {
        Self::new(FramePace::Wait)
    }
}

impl FrameRequest {
    /// A request that falls back to `baseline` once read.
    pub fn new(baseline: FramePace) -> Self {
        Self {
            baseline,
            pending: baseline,
        }
    }

    /// Raises the pace for this frame. Never lowers it.
    pub fn request(&mut self, pace: FramePace) {
        self.pending = self.pending.most_urgent(pace);
    }

    /// Raises the pace on the resource if it is present.
    ///
    /// Systems that only want to say "keep drawing" shouldn't have to
    /// care whether the app opted into idling at all.
    pub fn raise(resources: &mut crate::resource::Resources, pace: FramePace) {
        if let Some(request) = resources.get_mut::<Self>() {
            request.request(pace);
        }
    }

    /// What this frame asked for, resetting to the baseline.
    pub fn take(&mut self) -> FramePace {
        std::mem::replace(&mut self.pending, self.baseline)
    }

    /// The pace reads fall back to.
    pub fn baseline(&self) -> FramePace {
        self.baseline
    }
}

/// A handle that wakes a sleeping main loop from any thread.
///
/// The wake is *sticky*: a wake that lands between the end of a frame
/// and the moment the runner decides to sleep is not lost, because the
/// runner clears the flag itself and finds it set. Without that, the
/// window between "frame done" and "now sleeping" would silently drop
/// requests — rarely, and only under load, which is the worst kind.
#[derive(Clone, Default)]
pub struct FrameWaker {
    inner: Arc<WakerInner>,
}

#[derive(Default)]
struct WakerInner {
    pending: AtomicBool,
    /// Set by the runner once it owns something that can interrupt a
    /// platform sleep (a winit `EventLoopProxy`). Absent before then, and
    /// in headless runners, where the loop never sleeps anyway.
    notify: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl FrameWaker {
    /// Requests one more frame, from wherever.
    pub fn wake(&self) {
        self.inner.pending.store(true, Ordering::Release);
        if let Ok(notify) = self.inner.notify.lock()
            && let Some(notify) = notify.as_ref()
        {
            notify();
        }
    }

    /// Installs the platform interrupt. Called by the runner.
    pub fn set_notify(&self, notify: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.inner.notify.lock() {
            *slot = Some(Box::new(notify));
        }
    }

    /// Clears and returns the pending flag. Called by the runner right
    /// before it commits to sleeping.
    pub fn take_pending(&self) -> bool {
        self.inner.pending.swap(false, Ordering::AcqRel)
    }
}

impl std::fmt::Debug for FrameWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameWaker")
            .field("pending", &self.inner.pending.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn continuous_beats_everything() {
        assert_eq!(
            FramePace::Wait.most_urgent(FramePace::Continuous),
            FramePace::Continuous
        );
        assert_eq!(
            FramePace::After(Duration::from_millis(1)).most_urgent(FramePace::Continuous),
            FramePace::Continuous
        );
    }

    #[test]
    fn shorter_deadline_wins_and_wait_never_lowers() {
        let short = Duration::from_millis(5);
        let long = Duration::from_millis(500);
        assert_eq!(
            FramePace::After(long).most_urgent(FramePace::After(short)),
            FramePace::After(short)
        );
        assert_eq!(
            FramePace::After(long).most_urgent(FramePace::Wait),
            FramePace::After(long)
        );
    }

    #[test]
    fn repaint_delay_maps_to_the_three_cases() {
        assert_eq!(
            FramePace::from_repaint_delay(Duration::ZERO),
            FramePace::Continuous
        );
        assert_eq!(
            FramePace::from_repaint_delay(Duration::MAX),
            FramePace::Wait
        );
        assert_eq!(
            FramePace::from_repaint_delay(Duration::from_millis(16)),
            FramePace::After(Duration::from_millis(16))
        );
    }

    #[test]
    fn take_resets_to_baseline() {
        let mut request = FrameRequest::new(FramePace::Wait);
        request.request(FramePace::Continuous);
        assert_eq!(request.take(), FramePace::Continuous);
        assert_eq!(request.take(), FramePace::Wait);
    }

    #[test]
    fn a_continuous_frame_survives_a_later_wait() {
        // One system animating outvotes every system that has nothing
        // to say — otherwise draw order would decide whether the UI
        // animates, which is not a thing anyone can debug.
        let mut request = FrameRequest::new(FramePace::Wait);
        request.request(FramePace::Continuous);
        request.request(FramePace::Wait);
        assert_eq!(request.take(), FramePace::Continuous);
    }

    #[test]
    fn a_spinning_baseline_never_falls_asleep() {
        let mut request = FrameRequest::new(FramePace::Continuous);
        assert_eq!(request.take(), FramePace::Continuous);
        request.request(FramePace::Wait);
        assert_eq!(request.take(), FramePace::Continuous);
    }

    #[test]
    fn wake_is_sticky_without_a_notify() {
        let waker = FrameWaker::default();
        assert!(!waker.take_pending());
        waker.wake();
        assert!(waker.take_pending());
        assert!(!waker.take_pending());
    }

    #[test]
    fn wake_reaches_the_installed_notify_from_another_thread() {
        let waker = FrameWaker::default();
        let hits = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&hits);
        waker.set_notify(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let remote = waker.clone();
        std::thread::spawn(move || remote.wake()).join().unwrap();

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(waker.take_pending());
    }
}
