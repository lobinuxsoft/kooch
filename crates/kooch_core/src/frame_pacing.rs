//! How hard the main loop should spin — and how to wake it once it stops.

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
    pub fn most_urgent(self, other: Self) -> Self {
        match (self, other) {
            (Self::Continuous, _) | (_, Self::Continuous) => Self::Continuous,
            (Self::After(a), Self::After(b)) => Self::After(a.min(b)),
            (Self::After(d), Self::Wait) | (Self::Wait, Self::After(d)) => Self::After(d),
            (Self::Wait, Self::Wait) => Self::Wait,
        }
    }

    /// Reads egui's `repaint_delay` as a pace.
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

    /// Blocks until someone calls [`wake`](Self::wake), or `timeout` elapses. `None` waits
    /// indefinitely.
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
