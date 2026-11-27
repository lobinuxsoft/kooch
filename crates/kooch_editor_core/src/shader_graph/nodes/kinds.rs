//! Which kinds of shader a node can be part of, and which menus offer it.

use super::Node;

impl Node {
    /// Whether a shader of `kind` has what it reads: a UI node has no world, a mesh no UI node.
    pub(crate) fn fits(&self, kind: &str) -> bool {
        // A function has no values of its own to edit. A texture is a module's, so it joins the
        // material of whichever shader uses the file.
        if kind == "subgraph" {
            return self.output_kind().is_none()
                && self.declared().is_none()
                && !matches!(self, Self::SceneColor);
        }
        if kind == "terrain" {
            return self.fits_planet();
        }
        match self {
            Self::Argument { .. } | Self::Result { .. } => false,
            Self::SphereBase
            | Self::PlaneBase
            | Self::PlanetDirection
            | Self::PlanetPosition
            | Self::Slope => false,
            // What the mesher put in a terrain's vertices, so a lit surface only.
            Self::TerrainLayer => !matches!(kind, "ui" | "sky" | "post_process"),
            Self::Subgraph { reads, .. }
            | Self::Loop { reads, .. }
            | Self::Function { reads, .. } => reads
                .as_deref()
                .is_none_or(|reads| (reads == "ui") == (kind == "ui")),
            Self::UiImage
            | Self::UiColor
            | Self::UiBorderColor
            | Self::UiSize
            | Self::Parallax
            | Self::ImageNormal
            | Self::UiPoint => kind == "ui",
            Self::WorldPosition
            | Self::WorldNormal
            | Self::ViewDirection
            | Self::Fresnel
            | Self::UnpackNormal
            | Self::Texture { .. }
            | Self::SceneColor => kind != "ui",
            _ => true,
        }
    }

    /// A planet is evaluated in compute, per sample: no mesh, no camera, no material, no clock.
    fn fits_planet(&self) -> bool {
        match self {
            Self::Subgraph { reads, .. }
            | Self::Loop { reads, .. }
            | Self::Function { reads, .. } => reads.is_none(),
            Self::Argument { .. }
            | Self::Result { .. }
            | Self::Uv
            | Self::WorldPosition
            | Self::WorldNormal
            | Self::ViewDirection
            | Self::Time
            | Self::Sway
            | Self::Panner
            | Self::PolarCoordinates
            | Self::Twirl
            | Self::RadialShear
            | Self::Spherize
            | Self::SceneColor
            | Self::Fresnel
            | Self::UnpackNormal
            | Self::Texture { .. }
            | Self::TerrainLayer
            | Self::UiImage
            | Self::UiColor
            | Self::UiBorderColor
            | Self::UiSize
            | Self::Parallax
            | Self::ImageNormal
            | Self::UiPoint => false,
            _ => self.declared().is_none(),
        }
    }

    /// Whether the menu of a graph that is, or is not, a subgraph offers it.
    pub(crate) fn offered(&self, in_subgraph: bool) -> bool {
        match self {
            Self::Argument { .. } | Self::Result { .. } => in_subgraph,
            _ => !in_subgraph || self.fits("subgraph"),
        }
    }
}
