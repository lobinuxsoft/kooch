//! Sun shadows — cascaded shadow maps (#476).

pub(crate) mod alpha;
mod atlas;
mod cascades;
mod cube;
pub mod pages;
mod pass;
mod point;
mod raster;
mod settings;
mod spot;

pub use alpha::{ALPHA_LAYERS, ALPHA_SIDE, ShadowAlpha, shadow_alpha_shader};
pub use atlas::{DEFAULT_CASCADE_SIZE, SHADOW_DEPTH_FORMAT, ShadowAtlas};
pub use cascades::{
    CASCADE_BLEND_FRACTION, CASCADE_COUNT, Cascade, build_cascades, frustum_corners,
    orthographic_rh_reverse_z, split_distances,
};
pub use cube::{DEFAULT_CUBE_SIZE, PointShadowCubes};
pub use pages::{
    CensusCamera, CensusFrame, CensusKind, CensusLight, ClipmapConfig, POOL_PAGES, POOL_PAGES_WIDE,
    PageCensus, PageConfig, WorldBox, census,
};
pub use pass::{PreparedShadows, ShadowPass};
pub use point::{
    CUBE_FACES, CUBE_STICKINESS, CubeKey, FACE_DIRECTIONS, InstanceBounds, POINT_SHADOW_NEAR_Z,
    PointShadowDraw, face_view_proj, light_scene_hash, point_shadow, select_point_casters,
};
pub use raster::ShadowRasterizer;
pub use settings::{
    DEFAULT_POINT_SHADOWS, DEFAULT_SHADOW_DISTANCE, ShadowSettings, point_shadows_from_environment,
};
pub use spot::{SPOT_SHADOW_NEAR_Z, SpotShadowDraw, spot_shadow};
