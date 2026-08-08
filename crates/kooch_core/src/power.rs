//! Power profile for battery-aware quality defaults (audit §I.4).
//!
//! The engine runs on handheld hardware (Steam Deck, OneXFly F1 Pro) where
//! battery budget is a hard constraint. Subsystems read [`PowerProfile`] from
//! [`Resources`](crate::resource::Resources) to decide whether to enable
//! expensive effects (volumetric clouds, SSR, TAA, DoF) or scale them down to
//! sustain 60 fps at ~15 W TDP.
//!
//! # Detection
//!
//! Auto-detection happens once at startup — dynamic battery-percent switching
//! is explicitly out of scope. The user can override via:
//! - The editor menu bar (Engine → Power Profile).
//! - `KOOCH_POWER_PROFILE=battery|plugged|balanced|debug` environment variable
//!   (takes precedence over auto-detection, useful for CI and remote testing).
//!
//! # Heuristics
//!
//! | Platform     | Rule                                                        |
//! |--------------|-------------------------------------------------------------|
//! | Steam Deck   | `$SteamDeck=1` present → `Battery` (unless AC online).      |
//! | Linux        | `/sys/class/power_supply/*/type = Mains` + `online = 1`.    |
//! | Windows      | Default `Plugged`. Auto-detect deferred (needs `windows-sys`). |
//! | Other        | Default `Plugged`.                                          |

use std::fs;
use std::path::Path;

/// Quality tier for subsystems. Set at startup; user-overridable.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerProfile {
    /// Desktop or handheld plugged to wall — max quality budget.
    #[default]
    Plugged,
    /// Handheld plugged or desktop on UPS — mid quality.
    Balanced,
    /// Handheld on battery — sustain 60 fps at ~15 W TDP. Minimum quality.
    Battery,
    /// Developer mode — validation layers, profiler HUD, full quality.
    Debug,
}

impl PowerProfile {
    /// Parses a case-insensitive string (for env var and CLI overrides).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plugged" => Some(Self::Plugged),
            "balanced" => Some(Self::Balanced),
            "battery" => Some(Self::Battery),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }

    /// Stable identifier for serialization and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plugged => "plugged",
            Self::Balanced => "balanced",
            Self::Battery => "battery",
            Self::Debug => "debug",
        }
    }
}

/// Detects the profile at startup. Env var `KOOCH_POWER_PROFILE` wins if set.
pub fn detect() -> PowerProfile {
    if let Some(forced) = std::env::var("KOOCH_POWER_PROFILE")
        .ok()
        .and_then(|s| PowerProfile::parse(&s))
    {
        tracing::info!(
            profile = forced.as_str(),
            "power profile forced via KOOCH_POWER_PROFILE"
        );
        return forced;
    }

    let is_deck = std::env::var_os("SteamDeck").is_some_and(|v| v == "1");
    let on_ac = ac_online();

    let profile = match (is_deck, on_ac) {
        (true, Some(true)) => PowerProfile::Balanced,
        (true, Some(false)) => PowerProfile::Battery,
        (true, None) => PowerProfile::Battery, // conservative default on Deck
        (false, Some(false)) => PowerProfile::Battery,
        (false, _) => PowerProfile::Plugged,
    };

    tracing::info!(
        profile = profile.as_str(),
        steam_deck = is_deck,
        ac_online = ?on_ac,
        "power profile detected"
    );
    profile
}

/// Returns `Some(true)` if any AC adapter reports online on Linux, `Some(false)`
/// if all report offline, and `None` when the sysfs tree is unreadable
/// (non-Linux, sandbox, etc).
fn ac_online() -> Option<bool> {
    let root = Path::new("/sys/class/power_supply");
    let entries = fs::read_dir(root).ok()?;
    let mut saw_ac = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = fs::read_to_string(path.join("type")).ok()?;
        if kind.trim() != "Mains" {
            continue;
        }
        saw_ac = true;
        if let Ok(online) = fs::read_to_string(path.join("online"))
            && online.trim() == "1"
        {
            return Some(true);
        }
    }
    if saw_ac { Some(false) } else { None }
}

#[cfg(test)]
mod tests;
