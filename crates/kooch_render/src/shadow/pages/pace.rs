//! Moving lamps redraw their pages on a budget (#1861): a lamp small on screen keeps the pose its
//! pages were drawn from for a few frames, as HDRP's shadow update budget and id Tech's distant
//! light throttling do. The pages and their sampling read this copy; the lighting reads the real
//! lights, so only the shadow lags.

use glam::Vec3;
use kooch_lighting::{GpuLight, LIGHT_KIND_DIRECTIONAL};

/// The lights as the shadow pages see them, and the buffer the page passes and the shading bind.
#[derive(Default)]
pub struct LampPace {
    posed: Vec<GpuLight>,
    buffer: Option<wgpu::Buffer>,
    frame: Option<u32>,
}

impl LampPace {
    /// Advances once per `frame`, however many cameras ask.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lights: &[GpuLight],
        eye: Vec3,
        frame: u32,
    ) {
        if self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        pace(&mut self.posed, lights, eye, frame);
        let records = self.posed.len().max(1);
        let bytes = (records * std::mem::size_of::<GpuLight>()) as u64;
        if self.buffer.as_ref().is_none_or(|b| b.size() < bytes) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shadow_page_lamps"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer
            && !self.posed.is_empty()
        {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.posed));
        }
    }

    /// The lights in buffer order, each lamp at the pose its pages were drawn from.
    pub fn lamps(&self) -> &[GpuLight] {
        &self.posed
    }

    /// `None` until the first update.
    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }
}

/// Takes each light's real record when it is due, and keeps the posed one otherwise. Anything but
/// the pose changing (range, kind, flags) makes it due at once.
pub(crate) fn pace(posed: &mut Vec<GpuLight>, lights: &[GpuLight], eye: Vec3, frame: u32) {
    posed.truncate(lights.len());
    for (index, light) in lights.iter().enumerate() {
        let Some(old) = posed.get_mut(index) else {
            posed.push(*light);
            continue;
        };
        let mut moved_only = *old;
        moved_only.position = light.position;
        moved_only.direction = light.direction;
        let every = interval(light, eye);
        // Offset by index so lamps sharing an interval spread over its frames.
        let due = moved_only != *light || (frame as u64 + index as u64) % every == 0;
        if due {
            *old = *light;
        }
    }
}

/// Frames between redraws: every frame while the lamp's reach is a large part of its distance,
/// up to every fourth when it is small on screen. `KOOCH_LAMP_PACE=off` redraws every frame.
pub(crate) fn interval(light: &GpuLight, eye: Vec3) -> u64 {
    if light.kind == LIGHT_KIND_DIRECTIONAL || !pace_enabled() {
        return 1;
    }
    let distance = Vec3::from(light.position).distance(eye);
    let reach = light.range / distance.max(1e-3);
    if !(reach < 0.5) {
        1
    } else if reach >= 0.2 {
        2
    } else {
        4
    }
}

fn pace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        let off = std::env::var("KOOCH_LAMP_PACE").is_ok_and(|v| v.trim() == "off");
        if off {
            tracing::info!("KOOCH_LAMP_PACE=off: every lamp redraws its pages every frame");
        }
        !off
    })
}

#[cfg(test)]
mod tests;
