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
