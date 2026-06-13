// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolve the shared registry directory (`<shared_home>/agents/registry/`).

use std::path::PathBuf;

/// Resolve `<shared_home>/agents/registry/`.
///
/// The registry is shared across every portable/installed version on
/// the same machine — it must NOT use the per-version data dir.
///
/// Resolution order:
/// 1. `AGENTMUX_HOME_OVERRIDE` — test-only escape hatch, matching
///    `agentmux-common::data_paths` convention.
/// 2. Walk up from `AGENTMUX_DATA_DIR` (per-version data dir set by
///    the launcher) by three levels: `data → versions/<v> → versions
///    → <shared_home>`. Robust against the launcher running without
///    a real home dir (CI).
/// 3. Fall back to `~/.agentmux/`.
///
/// Returns `None` only if every source fails — caller treats this as
/// "registry disabled," no write attempts.
pub fn resolve_shared_registry_dir() -> Option<PathBuf> {
    resolve_shared_home().map(|h| h.join("agents").join("registry"))
}

/// Resolve the GLOBAL `<home>/shared/agents/definitions/` directory.
///
/// Unlike [`resolve_shared_registry_dir`] (channel-scoped — its
/// `AGENTMUX_DATA_DIR` walk-up lands on `channels/<ch>`), the definition
/// store is **channel-independent**: it lives under the global
/// `~/.agentmux/shared/` so an agent created in one channel is visible in
/// every channel (cross-channel agent persistence, P0.2). Resolves via the
/// launcher-exported `AGENTMUX_SHARED_DIR`, with a test override and a
/// `~/.agentmux/shared` fallback.
pub fn resolve_shared_definitions_dir() -> Option<PathBuf> {
    resolve_global_shared_root().map(|h| h.join("agents").join("definitions"))
}

/// The global, channel-independent `<home>/shared` root.
fn resolve_global_shared_root() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("AGENTMUX_HOME_OVERRIDE") {
        if !s.is_empty() {
            // Consistent with data_paths everywhere else: the override is the
            // ~/.agentmux root, with the shared dir at root/shared. (reagent
            // P2 on #1385.)
            return Some(PathBuf::from(s).join("shared"));
        }
    }
    if let Ok(s) = std::env::var("AGENTMUX_SHARED_DIR") {
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    dirs::home_dir().map(|h| h.join(".agentmux").join("shared"))
}

fn resolve_shared_home() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("AGENTMUX_HOME_OVERRIDE") {
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    if let Ok(s) = std::env::var("AGENTMUX_DATA_DIR") {
        if !s.is_empty() {
            let p = PathBuf::from(s);
            // .../versions/<v>/data → ancestors()[3] = .../
            if let Some(root) = p.ancestors().nth(3) {
                if !root.as_os_str().is_empty() {
                    return Some(root.to_path_buf());
                }
            }
        }
    }
    dirs::home_dir().map(|h| h.join(".agentmux"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Process-global env access — serialize so parallel tests don't race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear() {
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        std::env::remove_var("AGENTMUX_DATA_DIR");
        std::env::remove_var("AGENTMUX_SHARED_DIR");
    }

    #[test]
    fn override_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        let r = resolve_shared_registry_dir().unwrap();
        assert_eq!(r, PathBuf::from("/tmp/test-home/agents/registry"));
        clear();
    }

    #[test]
    fn walks_up_from_data_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var(
            "AGENTMUX_DATA_DIR",
            "/home/user/.agentmux/versions/0.33.822/data",
        );
        let r = resolve_shared_registry_dir().unwrap();
        assert_eq!(r, PathBuf::from("/home/user/.agentmux/agents/registry"));
        clear();
    }

    #[test]
    fn falls_back_to_home() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // No env set — uses dirs::home_dir(). Just assert non-None.
        assert!(resolve_shared_registry_dir().is_some());
    }

    #[test]
    fn definitions_dir_uses_shared_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_SHARED_DIR", "/home/user/.agentmux/shared");
        let r = resolve_shared_definitions_dir().unwrap();
        assert_eq!(
            r,
            PathBuf::from("/home/user/.agentmux/shared/agents/definitions")
        );
        clear();
    }

    #[test]
    fn definitions_dir_override_wins() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", "/tmp/test-home");
        let r = resolve_shared_definitions_dir().unwrap();
        // Override is the ~/.agentmux root; shared lives at root/shared.
        assert_eq!(
            r,
            PathBuf::from("/tmp/test-home/shared/agents/definitions")
        );
        clear();
    }
}
