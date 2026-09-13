//! The play-mode transform pull, taken off the editor's thread (#1014).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use kooch_remote::{MovedUpdate, RemoteClient};

/// How many replies may wait for the editor before the worker parks.
const INBOX_CAP: usize = 4;

/// How long the worker waits before retrying a failed pull.
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// One completed pull, in the order the project answered.
pub enum Pulled {
    /// What moved, or the project declining the question.
    Update(MovedUpdate),
    /// The exchange failed. Carries what to show as the stale reason.
    Failed(String),
}

/// Replies waiting for the editor, and the flags the worker obeys.
struct Inbox {
    replies: Mutex<VecDeque<Pulled>>,
    /// Signalled when the editor drains, resumes, or shuts down.
    room: Condvar,
    /// The worker pulls while true, parks while false.
    running: AtomicBool,
    /// One-way: set at shutdown, never cleared.
    stop: AtomicBool,
}

/// A worker that keeps the transform delta fresh without the editor
/// waiting for it.
pub struct MovedPump {
    inbox: Arc<Inbox>,
}

impl MovedPump {
    /// Starts a worker pulling from `client`, parked until
    /// [`Self::set_running`] turns it on.
    pub fn spawn(client: Arc<RemoteClient>) -> Self {
        let inbox = Arc::new(Inbox {
            replies: Mutex::new(VecDeque::with_capacity(INBOX_CAP)),
            room: Condvar::new(),
            running: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&inbox);
        std::thread::Builder::new()
            .name("kooch-moved-pump".to_owned())
            .spawn(move || pull_loop(&client, &worker))
            .expect("the editor could not start its remote pull thread");
        Self { inbox }
    }

    /// Turns the pull on or off.
    pub fn set_running(&self, running: bool) {
        if self.inbox.running.swap(running, Ordering::Release) != running {
            self.inbox.room.notify_all();
        }
    }

    /// Moves every reply the worker has finished into `out`, oldest first, and returns without
    /// blocking.
    pub fn drain(&self, out: &mut Vec<Pulled>) {
        let mut replies = self
            .inbox
            .replies
            .lock()
            .expect("moved pump inbox poisoned");
        if replies.is_empty() {
            return;
        }
        out.extend(replies.drain(..));
        drop(replies);
        self.inbox.room.notify_all();
    }
}

impl Drop for MovedPump {
    /// Signals the worker and returns — deliberately without joining.
    fn drop(&mut self) {
        self.inbox.stop.store(true, Ordering::Release);
        self.inbox.room.notify_all();
    }
}

/// The worker body: park until wanted, pull, hand the reply over.
fn pull_loop(client: &RemoteClient, inbox: &Inbox) {
    // Owned here rather than by the editor: the worker is the one making
    // the calls, so it is the only place that can chain a reply's
    // revision onto the next request without a frame of it going stale.
    let mut since = None;
    while wait_for_turn(inbox) {
        let pulled = match client.list_moved_since(since) {
            Ok(update) => {
                since = Some(update.revision);
                Pulled::Update(update)
            }
            Err(e) => {
                // The next pull has to be a full one: a revision the
                // project may never have issued would be answered
                // against a world this side cannot name.
                since = None;
                std::thread::sleep(RETRY_DELAY);
                Pulled::Failed(e.to_string())
            }
        };
        let mut replies = match inbox.replies.lock() {
            Ok(replies) => replies,
            // The editor panicked holding the inbox. There is no one
            // left to pull for.
            Err(_) => return,
        };
        replies.push_back(pulled);
    }
}

/// Blocks until there is a reason to pull, or `false` to shut down.
fn wait_for_turn(inbox: &Inbox) -> bool {
    let mut replies = match inbox.replies.lock() {
        Ok(replies) => replies,
        Err(_) => return false,
    };
    loop {
        if inbox.stop.load(Ordering::Acquire) {
            return false;
        }
        if inbox.running.load(Ordering::Acquire) && replies.len() < INBOX_CAP {
            return true;
        }
        replies = match inbox.room.wait(replies) {
            Ok(replies) => replies,
            Err(_) => return false,
        };
    }
}

#[cfg(test)]
mod tests;
