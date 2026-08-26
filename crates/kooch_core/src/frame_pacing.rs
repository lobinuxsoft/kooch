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

use std::sync::{Arc, Condvar, Mutex};
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
///
/// There are two ways for a runner to stop:
///
/// - **Under a window**, the platform event loop does the sleeping and
///   [`set_notify`](Self::set_notify) hands it the interrupt — for winit,
///   an `EventLoopProxy`, the one API documented as callable from another
///   thread.
/// - **Headless**, there is no event loop to sleep in, so
///   [`wait`](Self::wait) blocks on a condvar here.
#[derive(Clone, Default)]
pub struct FrameWaker {
    inner: Arc<WakerInner>,
}

#[derive(Default)]
struct WakerInner {
    /// Whether a frame has been asked for since the runner last looked.
    pending: Mutex<bool>,
    /// Signalled on every wake, for the headless runner parked in
    /// [`FrameWaker::wait`].
    woken: Condvar,
    /// Set by the runner once it owns something that can interrupt a
    /// platform sleep. Absent headless, where `wait` does the sleeping.
    notify: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl FrameWaker {
    /// Requests one more frame, from wherever.
    pub fn wake(&self) {
        if let Ok(mut pending) = self.inner.pending.lock() {
            *pending = true;
        }
        self.inner.woken.notify_all();
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
        match self.inner.pending.lock() {
            Ok(mut pending) => std::mem::replace(&mut *pending, false),
            Err(_) => false,
        }
    }

    /// Blocks until someone calls [`wake`](Self::wake), or `timeout`
    /// elapses. `None` waits indefinitely.
    ///
    /// Returns whether a wake actually arrived, as opposed to the
    /// deadline passing. Clears the pending flag either way: this *is*
    /// the runner looking.
    ///
    /// A wake that landed before the call returns immediately — the flag
    /// is checked before parking, which is what keeps a request that
    /// arrived mid-frame from being slept through.
    pub fn wait(&self, timeout: Option<Duration>) -> bool {
        let Ok(mut pending) = self.inner.pending.lock() else {
            // A poisoned lock means something already panicked; spinning
            // is better than deadlocking the loop that would report it.
            return false;
        };
        if std::mem::replace(&mut *pending, false) {
            return true;
        }

        match timeout {
            Some(timeout) => {
                let Ok((mut pending, _)) =
                    self.inner
                        .woken
                        .wait_timeout_while(pending, timeout, |pending| !*pending)
                else {
                    return false;
                };
                std::mem::replace(&mut *pending, false)
            }
            None => {
                let Ok(mut pending) = self.inner.woken.wait_while(pending, |pending| !*pending)
                else {
                    return false;
                };
                std::mem::replace(&mut *pending, false)
            }
        }
    }
}

impl std::fmt::Debug for FrameWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameWaker")
            .field(
                "pending",
                &self.inner.pending.lock().map(|p| *p).unwrap_or(false),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
