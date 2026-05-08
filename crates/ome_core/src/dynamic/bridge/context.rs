use crate::resource::Resources;
use crate::schedule::Schedule;

/// Engine-side context behind the opaque `EngineApi::ctx` pointer.
///
/// Created on the stack for each plugin interaction:
/// - During `OmePlugin::build()`: both `resources` and `schedule` are set
/// - During system callbacks: only `resources` is set (`schedule` is null)
pub struct BridgeContext {
    pub resources: *mut Resources,
    pub schedule: *mut Schedule,
}
