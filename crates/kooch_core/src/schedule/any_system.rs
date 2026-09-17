use super::identity::{SystemKey, SystemSource};
use super::order::Order;
use super::system_scope::{ScopeGuard, SystemScope};
use crate::resource::Resources;
use crate::system::{GpuSystem, System};

/// A registered system, with the profiling scope that carries its name.
pub(super) struct AnySystem {
    kind: Kind,
    scope: SystemScope,
    /// Which half of the build scheduled it, recorded here because the
    /// plugin that added it is the only thing that knows.
    source: SystemSource,
    /// What a toggle addresses it by. Built once at registration, so the
    /// per-frame check is a lookup rather than string work.
    key: SystemKey,
    /// Where in its stage it runs (#392).
    order: Order,
}

/// Either a CPU or GPU system.
pub(super) enum Kind {
    Cpu(Box<dyn System>),
    Gpu(Box<dyn GpuSystem>),
}

impl AnySystem {
    pub(super) fn cpu(
        system: Box<dyn System>,
        source: SystemSource,
        key: SystemKey,
        order: Order,
    ) -> Self {
        Self {
            kind: Kind::Cpu(system),
            scope: SystemScope::default(),
            source,
            key,
            order,
        }
    }

    pub(super) fn gpu(
        system: Box<dyn GpuSystem>,
        source: SystemSource,
        key: SystemKey,
        order: Order,
    ) -> Self {
        Self {
            kind: Kind::Gpu(system),
            scope: SystemScope::default(),
            source,
            key,
            order,
        }
    }

    pub(super) fn order(&self) -> &Order {
        &self.order
    }

    pub(super) fn source(&self) -> SystemSource {
        self.source
    }

    pub(super) fn key(&self) -> &SystemKey {
        &self.key
    }

    pub(super) fn name(&self) -> &str {
        match &self.kind {
            Kind::Cpu(s) => s.name(),
            Kind::Gpu(s) => s.name(),
        }
    }

    pub(super) fn is_gpu(&self) -> bool {
        matches!(self.kind, Kind::Gpu(_))
    }

    /// The GPU half, for the batch that shares one encoder.
    pub(super) fn as_gpu(&mut self) -> Option<&mut Box<dyn GpuSystem>> {
        match &mut self.kind {
            Kind::Gpu(s) => Some(s),
            Kind::Cpu(_) => None,
        }
    }

    /// Runs a CPU system inside its own scope. A no-op for a GPU one,
    /// which the batch runs instead.
    pub(super) fn run_cpu(&mut self, resources: &mut Resources) {
        let Kind::Cpu(system) = &mut self.kind else {
            return;
        };
        // Held for exactly the run: the guard closes on drop, and
        // nothing between here and `system.run` belongs to the system.
        let _scope = self.scope.enter(system.name());
        system.run(resources);
    }

    /// Opens this system's scope around whatever the caller does next.
    pub(super) fn scope(&mut self) -> Option<ScopeGuard> {
        match &self.kind {
            Kind::Cpu(s) => self.scope.enter(s.name()),
            Kind::Gpu(s) => self.scope.enter(s.name()),
        }
    }
}
