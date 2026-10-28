//! A cargo run that can be stopped whole: cargo, every `rustc` it started and the linker (#1358).
//!
//! Killing cargo's pid alone leaves its children reparented to init, finishing the link and holding
//! the `target/` lock. So cargo leads its own process group (a Job Object on Windows) and a stop is
//! sent to all of it.

use std::io;
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus};
use std::time::{Duration, Instant};

use process_wrap::std::{ChildWrapper, CommandWrap};

/// How long the tree gets to stop on SIGTERM before it is killed.
const GRACE: Duration = Duration::from_secs(2);

/// A running cargo and everything under it.
pub(super) struct CargoRun {
    child: Box<dyn ChildWrapper>,
    /// When a stop was asked for, so a tree ignoring SIGTERM is killed after [`GRACE`].
    stopping: Option<Instant>,
    killed: bool,
}

impl CargoRun {
    pub fn spawn(command: Command) -> io::Result<Self> {
        let mut wrap = CommandWrap::from(command);
        #[cfg(unix)]
        wrap.wrap(process_wrap::std::ProcessGroup::leader());
        #[cfg(windows)]
        wrap.wrap(process_wrap::std::JobObject);
        Ok(Self {
            child: wrap.spawn()?,
            stopping: None,
            killed: false,
        })
    }

    pub fn stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout().take()
    }

    pub fn stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr().take()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Asks the whole tree to stop. SIGTERM first, so cargo does not leave half-written artifacts.
    pub fn stop(&mut self) {
        if self.stopping.is_some() {
            return;
        }
        self.stopping = Some(Instant::now());
        #[cfg(unix)]
        let _ = self.child.signal(nix::sys::signal::Signal::SIGTERM as i32);
        // Windows has no polite stop for a tree: terminating the job is the stop.
        #[cfg(windows)]
        self.kill();
    }

    /// Whether nothing of the tree is left. Kills it once [`GRACE`] has passed.
    pub fn gone(&mut self) -> bool {
        // Reaps cargo, so its pid stops counting as alive.
        let _ = self.child.try_wait();
        if !self.alive() {
            return true;
        }
        if self.stopping.is_some_and(|at| at.elapsed() >= GRACE) {
            self.kill();
        }
        false
    }

    /// Stops the tree and blocks until it is gone, for a job dropped mid-build.
    pub fn stop_blocking(&mut self) {
        self.stop();
        let deadline = Instant::now() + GRACE * 2;
        while !self.gone() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn kill(&mut self) {
        if !self.killed {
            self.killed = true;
            let _ = self.child.start_kill();
        }
    }

    /// cargo's children outlive it and are not ours to reap, so the group is asked rather than
    /// waited on: signal 0 to the group fails with ESRCH once its last member is gone.
    #[cfg(unix)]
    fn alive(&self) -> bool {
        let group = nix::unistd::Pid::from_raw(self.child.id() as i32);
        nix::sys::signal::killpg(group, None).is_ok()
    }

    /// The Job Object took the whole tree with it when cargo's exit was observed.
    #[cfg(windows)]
    fn alive(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

#[cfg(all(test, unix))]
mod tests;
