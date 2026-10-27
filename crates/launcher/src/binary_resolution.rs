// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// Find the CEF host binary in the runtime directory.
/// Tries versioned name first (agentmux-X.Y.Z.exe), then any other
/// agentmux-<digit>... name (an older versioned host), then the old
/// agentmux-cef-X.Y.Z.exe pattern for backwards compat, then plain
/// agentmux-cef.exe (dev mode).
pub(crate) fn find_cef_binary(runtime_dir: &std::path::Path) -> std::path::PathBuf {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };

    let versioned = format!("agentmux-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_path = runtime_dir.join(&versioned);
    if versioned_path.exists() {
        return versioned_path;
    }

    if let Ok(entries) = std::fs::read_dir(runtime_dir) {
        let prefix = "agentmux-";
        let cef_prefix = "agentmux-cef";
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix)
                // Only a versioned name (agentmux-0.59.9) is a host. Anything
                // else next to the host is a helper or a package script: the
                // .deb and .rpm ship agentmux-install-update in this same
                // directory, and with no digit test it was the one name left
                // after the three exclusions below, so the launcher started
                // it as the window and gave up after three exits (v0.60.3).
                && name[prefix.len()..].starts_with(|c: char| c.is_ascii_digit())
                && !name.starts_with(cef_prefix)
                && !name.starts_with("agentmux-srv")
                // CRITICAL for the flat dev layout (macOS/Linux Phase 1):
                // launcher + host + srv share one dir, so the launcher
                // binary itself matches `agentmux-*`. Without this guard
                // the launcher resolves ITSELF as the host and spawns a
                // recursive launcher fork bomb. On Windows the launcher
                // lives at the root (not in runtime/), so this is a no-op.
                && !name.starts_with("agentmux-launcher")
                && name.ends_with(ext)
            {
                return entry.path();
            }
        }
    }

    let versioned_old = format!("agentmux-cef-{}{}", env!("CARGO_PKG_VERSION"), ext);
    let versioned_old_path = runtime_dir.join(&versioned_old);
    if versioned_old_path.exists() {
        return versioned_old_path;
    }

    runtime_dir.join(format!("agentmux-cef{}", ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &std::path::Path, name: &str) {
        std::fs::write(dir.join(name), b"").unwrap();
    }

    /// The host's name on this platform: Windows falls back to `agentmux-cef.exe`.
    fn host_name() -> String {
        format!("agentmux-cef{}", if cfg!(target_os = "windows") { ".exe" } else { "" })
    }

    /// The installed .deb/.rpm layout: host, launcher, srv and the upgrade
    /// helper side by side. The helper must never be picked as the host.
    #[test]
    fn a_package_helper_next_to_the_host_is_not_the_host() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["agentmux-cef", "agentmux-launcher", "agentmux-install-update", "agentmux-srv-0.60.3-linux.x64"] {
            touch(dir.path(), n);
        }
        assert_eq!(find_cef_binary(dir.path()), dir.path().join(host_name()));
    }

    #[test]
    fn any_agentmux_helper_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["agentmux-cef", "agentmux-bashwrap", "agentmux-mcp", "agentmux-install-update"] {
            touch(dir.path(), n);
        }
        assert_eq!(find_cef_binary(dir.path()), dir.path().join(host_name()));
    }

    #[test]
    fn an_older_versioned_host_is_still_found() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["agentmux-launcher", "agentmux-0.50.1", "agentmux-install-update"] {
            touch(dir.path(), n);
        }
        let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
        let want = format!("agentmux-0.50.1{ext}");
        // On Windows the name needs the extension to match the scan.
        if ext == ".exe" {
            touch(dir.path(), &want);
        }
        assert_eq!(find_cef_binary(dir.path()), dir.path().join(want));
    }

    #[test]
    fn the_exact_versioned_name_wins() {
        let dir = tempfile::tempdir().unwrap();
        let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
        let exact = format!("agentmux-{}{ext}", env!("CARGO_PKG_VERSION"));
        for n in [exact.as_str(), "agentmux-0.50.1", "agentmux-cef"] {
            touch(dir.path(), n);
        }
        assert_eq!(find_cef_binary(dir.path()), dir.path().join(exact));
    }
}
