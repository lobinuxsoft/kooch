//! A bind group kept between frames while it binds the same resources (#1869).
//!
//! A pass that rebuilds its group every frame pays the creation and, worse, makes wgpu track and
//! later drop one more object per submit. [`KeptGroups::get`] takes the same descriptor the pass
//! already builds and only creates a group when a resource in it was replaced.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::{Mutex, PoisonError};

/// One resource as the group bound it; wgpu resources compare by identity.
enum Bound {
    Buffer(wgpu::Buffer, u64, Option<NonZeroU64>),
    Sampler(wgpu::Sampler),
    View(wgpu::TextureView),
}

impl Bound {
    /// `None` for the array and acceleration-structure bindings: a group with one is never kept.
    fn of(resource: &wgpu::BindingResource<'_>) -> Option<Self> {
        match resource {
            wgpu::BindingResource::Buffer(binding) => Some(Self::Buffer(
                binding.buffer.clone(),
                binding.offset,
                binding.size,
            )),
            wgpu::BindingResource::Sampler(sampler) => Some(Self::Sampler((*sampler).clone())),
            wgpu::BindingResource::TextureView(view) => Some(Self::View((*view).clone())),
            _ => None,
        }
    }

    fn matches(&self, resource: &wgpu::BindingResource<'_>) -> bool {
        match (self, resource) {
            (Self::Buffer(buffer, offset, size), wgpu::BindingResource::Buffer(binding)) => {
                buffer == binding.buffer && *offset == binding.offset && *size == binding.size
            }
            (Self::Sampler(sampler), wgpu::BindingResource::Sampler(other)) => sampler == *other,
            (Self::View(view), wgpu::BindingResource::TextureView(other)) => view == *other,
            _ => false,
        }
    }
}

struct Kept {
    layout: wgpu::BindGroupLayout,
    entries: Vec<(u32, Bound)>,
    group: wgpu::BindGroup,
    /// The call to its key that last returned it.
    used: u64,
}

/// One label and slot's groups, with how many times it was asked for.
#[derive(Default)]
struct Slot {
    calls: u64,
    groups: Vec<Kept>,
}

/// Calls a group survives unused: a pass binding a set per material, or a ping-pong history, finds
/// each again, and a replaced resource is let go this many calls after its successor is bound.
const WINDOW: u64 = 16;

/// The bind groups built through it, by label and slot, and what each bound.
#[derive(Default)]
pub struct KeptGroups(Mutex<HashMap<(u64, u32), Slot>>);

impl KeptGroups {
    /// The group `descriptor` describes: a kept one when it binds the same resources, a new one
    /// otherwise.
    pub fn get(
        &self,
        device: &wgpu::Device,
        descriptor: &wgpu::BindGroupDescriptor<'_>,
    ) -> wgpu::BindGroup {
        self.get_at(device, 0, descriptor)
    }

    /// [`Self::get`] for a pass that binds several sets under one label, one `slot` each.
    pub fn get_at(
        &self,
        device: &wgpu::Device,
        slot: u32,
        descriptor: &wgpu::BindGroupDescriptor<'_>,
    ) -> wgpu::BindGroup {
        let mut kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = kept.entry((label_key(descriptor.label), slot)).or_default();
        slot.calls += 1;
        let calls = slot.calls;
        slot.groups.retain(|kept| calls - kept.used <= WINDOW);
        if let Some(found) = slot.groups.iter_mut().find(|kept| same(kept, descriptor)) {
            found.used = calls;
            return found.group.clone();
        }
        let group = device.create_bind_group(descriptor);
        let entries: Option<Vec<(u32, Bound)>> = descriptor
            .entries
            .iter()
            .map(|entry| Some((entry.binding, Bound::of(&entry.resource)?)))
            .collect();
        if let Some(entries) = entries {
            slot.groups.push(Kept {
                layout: descriptor.layout.clone(),
                entries,
                group: group.clone(),
                used: calls,
            });
        }
        group
    }
}

fn label_key(label: wgpu::Label<'_>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&label, &mut hasher);
    std::hash::Hasher::finish(&hasher)
}

fn same(kept: &Kept, descriptor: &wgpu::BindGroupDescriptor<'_>) -> bool {
    kept.layout == *descriptor.layout
        && kept.entries.len() == descriptor.entries.len()
        && kept
            .entries
            .iter()
            .zip(descriptor.entries)
            .all(|((binding, bound), entry)| {
                *binding == entry.binding && bound.matches(&entry.resource)
            })
}
