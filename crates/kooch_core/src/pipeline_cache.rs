//! Pipeline cache persistence (audit §H.2).

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::PathBuf;

use wgpu::{Adapter, Device, PipelineCache, PipelineCacheDescriptor};

const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the directory where pipeline cache blobs are stored.
fn cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg).join("kooch").join("pipeline_cache"));
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(local).join("kooch").join("pipeline_cache"));
    }
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".cache")
            .join("kooch")
            .join("pipeline_cache")
    })
}

fn adapter_hash(adapter: &Adapter) -> String {
    let info = adapter.get_info();
    let mut hasher = DefaultHasher::new();
    info.name.hash(&mut hasher);
    info.driver_info.hash(&mut hasher);
    ENGINE_VERSION.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn cache_path(adapter: &Adapter) -> Option<PathBuf> {
    cache_dir().map(|d| d.join(format!("{}.bin", adapter_hash(adapter))))
}

/// Attempts to create a [`PipelineCache`] for the device, seeded from disk if
/// a blob exists. Returns `None` if the adapter lacks
/// [`wgpu::Features::PIPELINE_CACHE`] or no cache directory can be resolved.
pub fn load(device: &Device, adapter: &Adapter) -> Option<PipelineCache> {
    if !adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
        tracing::debug!("adapter lacks PIPELINE_CACHE feature; pipelines will compile cold");
        return None;
    }
    let path = cache_path(adapter)?;
    let data = fs::read(&path).ok();

    // `fallback: true` makes the driver reject a stale/corrupt blob and return an empty cache
    // instead of producing undefined behavior.
    let cache = unsafe {
        device.create_pipeline_cache(&PipelineCacheDescriptor {
            label: Some("kooch_pipeline_cache"),
            data: data.as_deref(),
            fallback: true,
        })
    };

    match &data {
        Some(bytes) => tracing::info!(
            path = %path.display(),
            bytes = bytes.len(),
            "pipeline cache loaded"
        ),
        None => tracing::info!(
            path = %path.display(),
            "pipeline cache cold-start (no prior blob)"
        ),
    }
    Some(cache)
}

/// Persists the current pipeline cache blob to disk using an atomic rename.
pub fn save(cache: &PipelineCache, adapter: &Adapter) -> io::Result<()> {
    let path = cache_path(adapter)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no cache directory resolvable"))?;
    let data = cache.get_data().ok_or_else(|| {
        io::Error::other("pipeline cache returned no data (driver may not support retrieval)")
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("bin.tmp");
    fs::write(&tmp, &data)?;
    fs::rename(&tmp, &path)?;
    tracing::info!(
        path = %path.display(),
        bytes = data.len(),
        "pipeline cache saved"
    );
    Ok(())
}
