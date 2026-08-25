//! One profiling scope per system, and why it is a struct rather than a
//! macro.
//!
//! # The problem this exists to solve
//!
//! Stages are profiled and the systems inside them are not, so a frame
//! that spends 92 ms somewhere reports `PreUpdate: 92 ms` and stops. The
//! editor's `PreUpdate` alone holds nine systems from five crates —
//! remote mirroring, physics sync, world streaming, asset scanning — and
//! "one of these nine" is not an answer anyone can act on.
//!
//! # Why `profiling::scope!` cannot do it
//!
//! `puffin` caches the `ScopeId` in a `static` belonging to the CALL
//! SITE and registers it with the first name that site ever saw. One
//! `scope!` inside the dispatch loop would therefore report every system
//! in the process under whichever one ran first — which is exactly the
//! trap [`run_staged!`] documents for the fourteen stages, and the reason
//! that macro is expanded once per stage by hand.
//!
//! Fourteen stages can be written out. Systems are registered at
//! runtime, by plugins, in numbers nobody knows at compile time, so
//! there is no call site to expand. The id has to be per SYSTEM, not per
//! site — which is what `register_user_scopes` is for and what this
//! caches.
//!
//! [`run_staged!`]: super::schedule

#[cfg(feature = "cpu-profiler")]
pub(super) use enabled::{ScopeGuard, SystemScope};

#[cfg(not(feature = "cpu-profiler"))]
pub(super) use disabled::{ScopeGuard, SystemScope};

#[cfg(feature = "cpu-profiler")]
mod enabled {
    /// What an open scope is: puffin's own RAII guard.
    pub(in crate::schedule) type ScopeGuard = puffin::ProfilerScope;

    /// A system's own `puffin` scope id, registered once and kept.
    #[derive(Default)]
    pub(in crate::schedule) struct SystemScope {
        /// `None` until the system first runs. Registering at `add_system`
        /// time would name scopes for systems that never execute, and the
        /// profiler is a picture of what ran.
        id: Option<puffin::ScopeId>,
    }

    impl SystemScope {
        /// Opens the scope, or returns `None` when profiling is off.
        ///
        /// The guard closes on drop, so the caller keeps it alive for
        /// exactly the system's run and nothing else.
        pub(in crate::schedule) fn enter(&mut self, name: &str) -> Option<ScopeGuard> {
            if !puffin::are_scopes_on() {
                return None;
            }
            let id = match self.id {
                Some(id) => id,
                None => {
                    let details = puffin::ScopeDetails::from_scope_name(name.to_owned());
                    // 🔴 Registration is by NAME and puffin de-duplicates
                    // on it, so two systems that share a name share a
                    // scope. That is the right failure: the alternative
                    // is two rows called the same thing, which reads as
                    // one system running twice.
                    let id = *puffin::GlobalProfiler::lock()
                        .register_user_scopes(&[details])
                        .first()?;
                    self.id = Some(id);
                    id
                }
            };
            Some(ScopeGuard::new(id, ""))
        }

        /// The id this system settled on, for the test that pins that
        /// two systems do not settle on the same one.
        #[cfg(test)]
        pub(in crate::schedule) fn id(&self) -> Option<puffin::ScopeId> {
            self.id
        }
    }
}

#[cfg(not(feature = "cpu-profiler"))]
mod disabled {
    /// What a build without a profiler carries: nothing.
    ///
    /// Zero-sized, so wrapping every system in one costs neither memory
    /// nor a branch — the `enter` below is a constant `None` the
    /// optimiser deletes along with the scope it would have held.
    #[derive(Default)]
    pub(in crate::schedule) struct SystemScope;

    /// Never constructed. It exists so both builds return the same
    /// `Option<ScopeGuard>` and the call site is one line either way.
    pub(in crate::schedule) struct ScopeGuard;

    impl SystemScope {
        #[inline]
        pub(in crate::schedule) fn enter(&mut self, _name: &str) -> Option<ScopeGuard> {
            None
        }
    }
}
