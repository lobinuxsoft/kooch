//! What a page pool would actually hold — the census (#866).

use glam::{Mat4, UVec3, Vec2, Vec3, Vec4};

use kooch_lighting::ClusterGrid;

use super::point::{CUBE_FACES, POINT_SHADOW_NEAR_Z, face_view_proj};

/// How a virtual shadow map is diced into pages.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PageConfig {
    /// A page's side, in texels. UE5 uses 128.
    pub page: u32,
    /// A local light's virtual map at level 0, in texels. UE5 uses
    /// 16384, of which a frame residents a few thousand pages.
    pub virtual_size: u32,
}

impl Default for PageConfig {
    /// Epic's, verified against the UE 5.8 documentation rather than quoted second-hand: *"they
    /// have a virtual resolution of 16k x 16k pixels"* and *"VSMs split the shadow map into tiles
    /// (or Pages) that are 128x128 each"*.
    fn default() -> Self {
        Self {
            page: 128,
            virtual_size: 16384,
        }
    }
}

/// The finest a LOCAL light's chain may go, in virtual texels across
/// one cube face. Mirror of `LOCAL_MAX_TEXELS` in `page_table.wgsl` —
/// see its doc for the measurement that set it.
pub const LOCAL_MAX_TEXELS: u32 = 2048;

/// Pages in Unreal's physical pool by default — `r.Shadow.Virtual.MaxPhysicalPages`.
pub const POOL_PAGES: u32 = 4096;

/// The open-world pool Epic recommends over the default.
pub const POOL_PAGES_WIDE: u32 = 6144;

impl PageConfig {
    /// Levels in the chain, level 0 being the finest.
    pub fn levels(&self) -> u32 {
        let side = self.side(0).max(1);
        side.ilog2() + 1
    }

    /// Pages along one side of `level`.
    pub fn side(&self, level: u32) -> u32 {
        (self.virtual_size >> level).div_ceil(self.page).max(1)
    }

    /// Texels along one side of `level`.
    pub fn texels(&self, level: u32) -> u32 {
        (self.virtual_size >> level).max(self.page)
    }

    /// Pages in one face's whole chain — the stride between faces in the
    /// census bitmap.
    pub fn face_pages(&self) -> u32 {
        (0..self.levels()).map(|l| self.side(l).pow(2)).sum()
    }

    /// The finest chain level a LOCAL light may use. Mirror of `local_level_floor` in
    /// `page_table.wgsl`, floor for floor: the GPU addresses a lamp's chain from here up, so a CPU
    /// that disagreed would size a table the shaders index past.
    pub fn local_floor(&self) -> u32 {
        let mut floor = 0;
        let mut texels = self.virtual_size;
        while texels > LOCAL_MAX_TEXELS {
            texels >>= 1;
            floor += 1;
        }
        floor
    }

    /// Pages in one face's chain from the floor up — a LOCAL light's
    /// face stride in the page table. Mirror of `local_face_pages` in
    /// `page_table.wgsl`.
    pub fn local_face_pages(&self) -> u32 {
        (self.local_floor()..self.levels())
            .map(|l| self.side(l).pow(2))
            .sum()
    }

    /// Where `level` starts inside one face's chain.
    fn level_base(&self, level: u32) -> u32 {
        (0..level).map(|l| self.side(l).pow(2)).sum()
    }

    /// What one page costs, at `Depth32Float`.
    pub fn page_bytes(&self) -> u64 {
        self.page as u64 * self.page as u64 * 4
    }
}

pub mod lamp_cull;
pub mod mark;
pub mod pool;
pub mod pyramid;
pub mod raster;

mod census;

pub use census::*;

#[cfg(test)]
mod tests;

/// The light set as the page cache has to see it: how many shadow casters the frame has, and
/// whether one of them is the sun.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Casters {
    /// Shadow-casting lights in the frame, the sun included.
    pub count: u32,
    /// Whether the frame has a shadow-casting directional light.
    pub sun: bool,
}

impl Casters {
    /// What a frame's extracted lights add up to.
    pub fn of_frame(frame: &kooch_lighting::LightFrame) -> Self {
        let sun = frame.sun().is_some();
        Self {
            count: u32::from(sun)
                + frame.point_shadows().len() as u32
                + frame.spot_shadows().len() as u32,
            sun,
        }
    }

    /// Nothing casts, so no page will ever be requested again.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether this frame lost a caster `before` had.
    pub fn lost(&self, before: Casters) -> bool {
        self.count < before.count || (before.sun && !self.sun)
    }
}
