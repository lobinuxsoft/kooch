use crate::system::{GpuSystem, System};

/// Either a CPU or GPU system.
pub(super) enum AnySystem {
    Cpu(Box<dyn System>),
    Gpu(Box<dyn GpuSystem>),
}

impl AnySystem {
    pub(super) fn name(&self) -> &str {
        match self {
            Self::Cpu(s) => s.name(),
            Self::Gpu(s) => s.name(),
        }
    }

    pub(super) fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }
}
