//! Comparing a bind mount's source as Docker reports it with the host path it was asked for.

use super::normalize_host_path;

/// Canonical form of a mount source for comparison.
///
/// Only Windows is folded, deliberately (#2933): Docker on Windows can report a
/// differently-cased drive letter than we asked for, and a case-sensitive compare there
/// produces phantom drift and a needless recreate. Folding unconditionally is the opposite bug:
/// on a case-sensitive host, switching an agent's workspace from `/work/Foo` to the genuinely
/// different `/work/foo` would look like a match, `ensure_running` would skip the recreate, and
/// `/workspace` would keep serving the old directory. Elsewhere only the slashes are normalized.
pub(super) fn fold_mount_source(source: &str) -> String {
    if cfg!(windows) {
        windows_source_key(source)
    } else {
        normalize_host_path(source)
    }
}

/// What a Windows host path, and the form Docker reports it in, have in common. Docker Desktop
/// runs containers in a VM and reports a bind source there (`/run/desktop/mnt/host/c/Users/x`;
/// `/mnt/c/...` and `/host_mnt/c/...` under WSL and older builds), while the source asked for is
/// the host's (`C:\Users\x`, `C:/Users/x`, or `\\?\C:\Users\x`). Without folding them together
/// every start reads as drift and recreates the container. Drive letters and case are folded
/// (NTFS is case-insensitive) and a trailing slash is dropped; a path that is no drive stays as
/// it is.
pub(super) fn windows_source_key(source: &str) -> String {
    let mut path = normalize_host_path(source);
    if let Some(rest) = path.strip_prefix("//?/") {
        path = rest.to_string();
    }
    for prefix in ["/run/desktop/mnt/host/", "/mnt/host/", "/host_mnt/", "/mnt/"] {
        let Some(rest) = path.strip_prefix(prefix) else { continue };
        let (drive, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if drive.len() == 1 && drive.chars().all(|c| c.is_ascii_alphabetic()) {
            path = format!("{drive}:/{tail}");
            break;
        }
    }
    let mut key = path.to_lowercase();
    while key.len() > 1 && key.ends_with('/') && !key.ends_with(":/") {
        key.pop();
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_path_and_every_form_docker_desktop_reports_it_in_are_one_source() {
        let want = windows_source_key(r"C:\Users\me\dir");
        for form in [
            "C:/Users/me/dir",
            "c:/users/me/dir",
            r"C:\Users\me/dir",
            r"\\?\C:\Users\me\dir",
            "/run/desktop/mnt/host/c/Users/me/dir",
            "/run/desktop/mnt/host/C/Users/me/dir",
            "/mnt/host/c/Users/me/dir",
            "/mnt/c/Users/me/dir",
            "/host_mnt/c/Users/me/dir",
            "C:/Users/me/dir/",
            r"C:\Users\me\dir\",
            "/run/desktop/mnt/host/c/Users/me/dir/",
        ] {
            assert_eq!(windows_source_key(form), want, "{form}");
        }
        assert_eq!(want, "c:/users/me/dir");
    }

    #[test]
    fn different_directories_and_drives_stay_different() {
        let base = windows_source_key("C:/Users/me/dir");
        for other in ["C:/Users/me/other", "D:/Users/me/dir", "/run/desktop/mnt/host/d/Users/me/dir", "/mnt/d/Users/me/dir", "C:/Users/me"] {
            assert_ne!(windows_source_key(other), base, "{other}");
        }
    }

    #[test]
    fn a_drive_root_and_a_path_that_is_no_drive_keep_their_shape() {
        assert_eq!(windows_source_key(r"C:\"), "c:/");
        assert_eq!(windows_source_key("/run/desktop/mnt/host/c"), "c:/");
        assert_eq!(windows_source_key("/run/desktop/mnt/host/c/"), "c:/");
        // `/mnt/<x>` that is a folder, not a drive letter.
        assert_eq!(windows_source_key("/mnt/data/x"), "/mnt/data/x");
        assert_eq!(windows_source_key("/mnt/ab/x"), "/mnt/ab/x");
        assert_eq!(windows_source_key("/"), "/");
        // A named volume's name passes through.
        assert_eq!(windows_source_key("agentmux-claude-agentmux-abc"), "agentmux-claude-agentmux-abc");
    }

    /// Case folding must be Windows-only: on a case-sensitive host `/work/Foo` and `/work/foo` are
    /// different directories, and folding them together would hide real drift and leave /workspace
    /// on the stale one. Docker Desktop's path form is not rewritten there either.
    #[test]
    fn mount_source_is_folded_only_where_the_host_is_windows() {
        let (a, b) = (fold_mount_source("/work/Foo"), fold_mount_source("/work/foo"));
        let (vm, host) = (fold_mount_source("/run/desktop/mnt/host/c/x"), fold_mount_source("C:/x"));
        if cfg!(windows) {
            assert_eq!(a, b, "windows paths are case-insensitive");
            assert_eq!(vm, host, "Docker Desktop's form is the host's path");
        } else {
            assert_ne!(a, b, "a case-sensitive host must see these as different mounts");
            assert_ne!(vm, host);
        }
        // Slash normalization is unconditional either way.
        assert_eq!(fold_mount_source(r"C:\a\b"), fold_mount_source("C:/a/b"));
    }

    /// The container Docker Desktop made for a Windows host's agent is the one `mounts_match` must accept,
    /// with its sources reported as the VM sees them.
    #[cfg(windows)]
    #[test]
    fn a_container_docker_desktop_made_for_this_spec_has_no_drift() {
        use super::super::{ContainerManager, ContainerMountSpec};
        use bollard::models::{ContainerInspectResponse, MountPoint};
        let spec = ContainerMountSpec {
            claude_config_host_dir: Some(r"C:\Users\me\.agentmux\shared\claude".to_string()),
            workspace_host_dir: Some(r"C:\Users\me\.agentmux/agents/a-1".to_string()),
            memory_host_dir: None,
            github_hooks_host_dir: None,
            ..Default::default()
        };
        let reported = |source: &str| match source.split_once(":/") {
            Some((drive, rest)) if drive.len() == 1 => format!("/run/desktop/mnt/host/{}/{rest}", drive.to_lowercase()),
            _ => source.to_string(),
        };
        let mounts = ContainerManager::desired_owned_mounts("agentmux-a-1", &spec)
            .into_iter()
            .map(|(target, source)| {
                let named = !source.contains('/');
                MountPoint {
                    destination: Some(target),
                    name: named.then(|| source.clone()),
                    source: Some(if named { format!("/var/lib/docker/volumes/{source}/_data") } else { reported(&source) }),
                    ..Default::default()
                }
            })
            .collect();
        let details = ContainerInspectResponse { mounts: Some(mounts), ..Default::default() };
        assert!(ContainerManager::mounts_match(&details, "agentmux-a-1", &spec));
        // A workspace that really changed is still drift.
        let moved = ContainerMountSpec { workspace_host_dir: Some(r"C:\Users\me\.agentmux/agents/a-2".to_string()), ..spec };
        assert!(!ContainerManager::mounts_match(&details, "agentmux-a-1", &moved));
    }
}
