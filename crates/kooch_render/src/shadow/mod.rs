//! Sun shadows — cascaded shadow maps (#476).
//!
//! Part of **Inti**, the lighting system, even though the code lives in
//! `kooch_render`: rasterising depth from a light needs the meshlet
//! pipeline, and `kooch_lighting` sits below it. The shading side — the
//! sampling, the filter — lives with the rest of the model in
//! `inti_pbr.wgsl`.
//!
//! # Why a cascade is not a `MeshletView`
//!
//! A `MeshletView` owns a visibility buffer, a colour target, a density
//! accumulator and a Hi-Z pyramid. A shadow cascade reads none of them:
//! it needs depth and nothing else. Four cascades as views would be
//! hundreds of megabytes of textures no shadow ever samples.
//!
//! What a cascade genuinely needs from a view is the **cull** — which
//! meshlets survive depends on where you are looking from, and a light
//! looks from somewhere else. `MeshletCull` is buffers rather than
//! textures, so that part is cheap to have one of per cascade.

mod atlas;
mod cascades;
mod cube;
mod pass;
mod point;
mod raster;
mod settings;
mod spot;

pub use atlas::{DEFAULT_CASCADE_SIZE, SHADOW_DEPTH_FORMAT, ShadowAtlas};
pub use cascades::{
    CASCADE_BLEND_FRACTION, CASCADE_COUNT, Cascade, build_cascades, frustum_corners,
    orthographic_rh_reverse_z, split_distances,
};
pub use cube::{DEFAULT_CUBE_SIZE, PointShadowCubes};
pub use pass::{PreparedShadows, ShadowPass};
pub use point::{
    CUBE_FACES, FACE_DIRECTIONS, POINT_SHADOW_NEAR_Z, PointShadowDraw, face_view_proj,
    point_shadow, select_point_casters,
};
pub use raster::ShadowRasterizer;
pub use settings::{DEFAULT_SHADOW_DISTANCE, ShadowSettings};
pub use spot::{SPOT_SHADOW_NEAR_Z, SpotShadowDraw, spot_shadow};
