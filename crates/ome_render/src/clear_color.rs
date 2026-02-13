//! Background clear color for the render pass.

/// The color used to clear the screen each frame.
///
/// Stored as a resource; the render system reads it every frame.
#[derive(Debug, Clone, Copy)]
pub struct ClearColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Default for ClearColor {
    fn default() -> Self {
        Self {
            r: 0.1,
            g: 0.1,
            b: 0.15,
            a: 1.0,
        }
    }
}

impl ClearColor {
    /// Converts to wgpu's color type.
    #[inline]
    pub fn to_wgpu(self) -> wgpu::Color {
        wgpu::Color {
            r: self.r,
            g: self.g,
            b: self.b,
            a: self.a,
        }
    }
}
