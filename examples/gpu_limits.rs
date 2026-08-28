//! What this machine's adapters actually offer, against what the engine
//! asks for.
//!
//! # Why it exists
//!
//! `elevated_compute_limits` names five limits, clamps each to the
//! adapter's and WARNS when clamped, saying which feature degrades.
//! Every other limit falls through to `wgpu::Limits::default()` — which
//! is a conservative floor, not this machine's ceiling, and nothing
//! says so.
//!
//! 🔴 `max_buffer_size` defaults to 256 MiB and
//! `max_storage_buffer_binding_size` to 128 MiB. A buffer past either
//! is not an error at creation: wgpu returns an INVALID buffer and
//! every submit afterwards fails validation naming a label and no
//! cause. That is what a 2.4 GB lamp arena looked like from the log.
//!
//! # 🔴 PER BUFFER, not a budget
//!
//! `max_buffer_size` is the largest a SINGLE buffer may be. It is not
//! a total: forty buffers of 200 MiB are fine and one of 300 MiB is
//! not. That is why a 2.4 GB arena failed on a card with 16 GB.
//!
//! ```bash
//! cargo run --example gpu_limits
//! ```

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    if adapters.is_empty() {
        println!("no adapters");
        return;
    }
    let asked = wgpu::Limits::default();
    for adapter in adapters {
        let info = adapter.get_info();
        let limits = adapter.limits();
        println!(
            "\n{} — {:?} / {:?}",
            info.name, info.device_type, info.backend
        );
        println!(
            "{:<44} {:>14} {:>14}",
            "limit", "the default", "this adapter"
        );
        for (name, default, offered) in [
            (
                "max_buffer_size",
                mib(asked.max_buffer_size),
                mib(limits.max_buffer_size),
            ),
            (
                "max_storage_buffer_binding_size",
                mib(asked.max_storage_buffer_binding_size as u64),
                mib(limits.max_storage_buffer_binding_size as u64),
            ),
            (
                "max_uniform_buffer_binding_size",
                mib(asked.max_uniform_buffer_binding_size as u64),
                mib(limits.max_uniform_buffer_binding_size as u64),
            ),
        ] {
            let note = if offered > default {
                "  ← headroom unused"
            } else {
                ""
            };
            println!("{name:<44} {default:>11.0} MiB {offered:>10.0} MiB{note}");
        }
        println!(
            "{:<44} {:>14} {:>14}",
            "max_compute_workgroups_per_dimension",
            asked.max_compute_workgroups_per_dimension,
            limits.max_compute_workgroups_per_dimension,
        );
        println!(
            "{:<44} {:>14} {:>14}",
            "max_storage_buffers_per_shader_stage",
            asked.max_storage_buffers_per_shader_stage,
            limits.max_storage_buffers_per_shader_stage,
        );
    }
    println!(
        "\n⚠️  Headroom is not a licence to spend it. A buffer that fits\n\
         this desktop and not the handheld is a crash somebody else finds.\n"
    );
}
