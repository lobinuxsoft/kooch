//! How a device-side array grows, and what one costs — the two answers every pool in the engine
//! was writing out by hand.

/// The capacity a buffer holding `required` slots should be given, never less than doubling.
///
/// 🔴 Geometric, not exact: a buffer sized to what this frame needs is reallocated by the next
/// frame that needs one more. The doubling floor is what stops a pool that grows by one from
/// reallocating every frame.
pub fn grown(required: u32, capacity: u32) -> u32 {
    required
        .checked_next_power_of_two()
        .unwrap_or(required)
        .max(capacity.saturating_mul(2))
}

/// What a set of buffers holds on the device.
///
/// 🔴 The buffers' own sizes, not the data in them. A pool carries slack and never shrinks, so
/// what it was asked to store is not what it took.
pub fn buffer_bytes(buffers: &[&wgpu::Buffer]) -> u64 {
    buffers.iter().map(|buffer| buffer.size()).sum()
}

/// What a texture holds on the device, across every mip and layer.
pub fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    let block = texture.format().block_copy_size(None).unwrap_or(4) as u64;
    let size = texture.size();
    let mips = (0..texture.mip_level_count()).map(|mip| {
        let w = (size.width >> mip).max(1) as u64;
        let h = (size.height >> mip).max(1) as u64;
        w * h * block
    });
    mips.sum::<u64>() * size.depth_or_array_layers as u64
}

#[cfg(test)]
mod tests;
