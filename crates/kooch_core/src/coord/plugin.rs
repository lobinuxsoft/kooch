//! [`OriginPlugin`] — registers the [`ActiveOrigin`] resource so the
//! rest of the engine can read the simulation frame's universe anchor.

use crate::app::App;
use crate::coord::ActiveOrigin;
use crate::plugin::Plugin;

/// Inserts [`ActiveOrigin`] (default = [`ActiveOrigin::ZERO`]) at app
/// startup. Add this plugin once near the bottom of `MinimalPlugins` /
/// `DefaultPlugins`; consumers (render pipelines, debug HUDs, etc.)
/// fetch the resource by `Resources::get::<ActiveOrigin>()`.
///
/// The rebase system that mutates [`ActiveOrigin`] in response to
/// player movement is filed as a follow-up to issue #50.
pub struct OriginPlugin;

impl Plugin for OriginPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ActiveOrigin::default());
    }

    fn name(&self) -> &str {
        "OriginPlugin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::UniverseCoord;

    #[test]
    fn plugin_inserts_active_origin_resource() {
        let mut app = App::new();
        app.add_plugin(OriginPlugin);
        // Resource is inserted at build time.
        let origin = app
            .resources()
            .get::<ActiveOrigin>()
            .expect("ActiveOrigin should be inserted by OriginPlugin");
        assert_eq!(origin.coord(), UniverseCoord::ZERO);
    }
}
