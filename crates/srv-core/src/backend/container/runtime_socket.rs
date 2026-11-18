// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! No container runtime socket, pipe or control folder is reachable through any mount of a container agent, or a folder that
//! holds one, whichever field the mount comes from (its volumes list, its workspace folder) and whoever wrote it (the
//! definition, a template, a block's launch settings, another channel's registry). Sources are
//! compared case-insensitively, with `\` as `/`, `.` and `..` resolved, and a link followed where the path exists.
//! [`refused_mount`] is asked once, before an existing container is reused or a new one made; definitions that come in from
//! outside (the shared registry, an old template) have the entries dropped ([`without_refused`]), and a definition saved with
//! one is refused ([`refused_in_json`]).

use std::path::PathBuf;

use crate::backend::cli_messaging::normalized;

/// A socket file or a named pipe whose name has one of these in it, and ends in `.sock`, is a runtime's.
const BRANDS: [&str; 5] = ["docker", "podman", "containerd", "crio", "buildkit"];
/// Names that are a runtime's pipe or socket whatever their folder.
const NAMES: [&str; 5] = ["docker_engine", "docker_cli", "dockerdesktoplinuxengine", "podman-machine-default", "docker_engine_linux"];
/// Where a source starting this way is a Windows named pipe.
const PIPES: [&str; 3] = ["//./pipe", "//?/pipe", "npipe:"];
/// Runtime sockets and folders on a Linux or macOS host, and inside Docker Desktop's and WSL's machines. A source is refused
/// when it is one, is inside one, or holds one.
const SYSTEM: &[&str] = &[
    "/var/run/docker.sock",
    "/run/docker.sock",
    "/var/run/docker",
    "/run/docker",
    "/var/run/podman",
    "/run/podman",
    "/var/run/containerd",
    "/run/containerd",
    "/var/run/crio",
    "/run/crio",
    "/run/k3s",
    "/run/buildkit",
    "/run/snap.docker",
    "/var/run/snap.docker",
    "/var/lib/docker",
    "/var/lib/containers",
    "/var/lib/containerd",
    "/var/lib/podman",
    "/var/snap/docker",
    "/mnt/wsl/docker-desktop",
    "/mnt/wsl/docker-desktop-bind-mounts",
    "/mnt/wsl/podman-sockets",
    "/var/run/cri-dockerd.sock",
    "/run/cri-dockerd.sock",
];
/// Under a user's home folder. A folder that holds one of these is refused only where the entry is there.
const HOME: &[&str] = &[
    ".docker/run",
    ".docker/desktop",
    ".colima",
    ".lima",
    ".rd",
    ".local/share/containers",
    ".local/share/docker",
    "library/containers/com.docker.docker",
    "appdata/local/docker",
];
/// Under the Windows data folders (`%PROGRAMDATA%`, `%LOCALAPPDATA%`), with the same rule.
const DATA: &[&str] = &["docker", "dockerdesktop", "docker desktop"];

/// The first of a container agent's volumes, or its workspace folder, that it may not mount, as the source (or, for a pipe,
/// the whole entry): a host folder the CLI keeps its sessions in, or a container runtime's socket, pipe or folder.
pub fn refused_mount(volumes: &[String], workspace: Option<&str>) -> Option<String> {
    let (homes, data) = (homes(), data_dirs());
    refused_in(volumes, &homes, &data).or_else(|| workspace.map(str::trim).filter(|w| !w.is_empty()).and_then(|w| refused_entry(w, w, &homes, &data)))
}

/// [`refused_mount`] for a volumes list alone.
pub fn refused_volume(volumes: &[String]) -> Option<String> {
    refused_mount(volumes, None)
}

/// The volumes of a definition's `container_volumes` (a JSON array of specs); nothing for text that is not one.
fn volumes_of(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// The first entry of a definition's `container_volumes` that a container may not mount.
pub fn refused_in_json(json: &str) -> Option<String> {
    refused_volume(&volumes_of(json))
}

/// A definition's `container_volumes` without the entries a container may not mount, for one that comes in from outside
/// this AgentMux. The entries dropped are logged; a list with none is returned as it was.
pub fn without_refused(json: &str) -> String {
    let all = volumes_of(json);
    let kept: Vec<&String> = all.iter().filter(|v| refused_volume(std::slice::from_ref(v)).is_none()).collect();
    if kept.len() == all.len() {
        return json.to_string();
    }
    tracing::warn!(dropped = all.len() - kept.len(), "a definition from outside listed a mount a container may not have (a container runtime's socket or folder); the entry is dropped");
    serde_json::to_string(&kept).unwrap_or_else(|_| "[]".to_string())
}

fn refused_in(volumes: &[String], homes: &[PathBuf], data: &[PathBuf]) -> Option<String> {
    volumes.iter().find_map(|spec| {
        let (source, _, _) = super::parse_volume_spec(spec)?;
        refused_entry(spec, source, homes, data)
    })
}

/// `spec` is the whole entry as written, `source` the host side of it.
fn refused_entry(spec: &str, source: &str, homes: &[PathBuf], data: &[PathBuf]) -> Option<String> {
    if crate::backend::cli_messaging::refused_volume_source(std::iter::once(source)).is_some() {
        return Some(source.to_string());
    }
    let raw = spec.replace('\\', "/").to_lowercase();
    if PIPES.iter().any(|p| raw.starts_with(p)) {
        // A pipe is named by its path; a form that does not split into a path (`npipe:`) by the whole entry.
        return Some(if source.starts_with(['/', '\\']) { source.to_string() } else { spec.to_string() });
    }
    (is_host_path(source) && names_runtime(source, homes, data)).then(|| source.to_string())
}

/// Whether a volume source is a path on the host (a bind mount), not a named volume.
fn is_host_path(source: &str) -> bool {
    let b = source.as_bytes();
    source.starts_with(['/', '\\', '~']) || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// A source as compared: the home folder in place of a leading `~`, then lower case and `/` separators, with no `.`, `..`
/// or doubled separator, and none of Windows' extended-path prefix.
fn comparable(source: &str, homes: &[PathBuf]) -> String {
    let source = source.trim();
    let expanded = match (source.strip_prefix('~'), homes.first()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => format!("{}{rest}", home.to_string_lossy()),
        _ => source.to_string(),
    };
    let flat = normalized(&expanded).to_lowercase();
    flat.strip_prefix("/?/").map_or(flat.clone(), str::to_string)
}

/// `p` is `dir`, or inside it.
fn inside(p: &str, dir: &str) -> bool {
    p == dir || p.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'))
}

/// `p` is a folder above `dir`.
fn above(p: &str, dir: &str) -> bool {
    dir.strip_prefix(p).is_some_and(|rest| rest.starts_with('/')) || (p == "/" && dir.starts_with('/'))
}

/// Whether a path or any component of it is a runtime's socket or pipe by name.
fn named_like_a_socket(p: &str) -> bool {
    p.split('/').any(|c| NAMES.contains(&c) || (c.ends_with(".sock") && BRANDS.iter().any(|b| c.contains(b))))
}

fn names_runtime(source: &str, homes: &[PathBuf], data: &[PathBuf]) -> bool {
    // The path as written, and where a link or a short name leads, when it exists on this host.
    let mut forms = vec![comparable(source, homes)];
    if let Ok(real) = std::fs::canonicalize(source.trim()) {
        forms.push(comparable(&real.to_string_lossy(), &[]));
    }
    let under_home = |rel: &str| homes.iter().map(|h| h.join(rel)).collect::<Vec<_>>();
    let user_entries: Vec<PathBuf> = HOME.iter().flat_map(|r| under_home(r)).chain(DATA.iter().flat_map(|r| data.iter().map(move |d| d.join(r)))).collect();
    forms.iter().any(|p| {
        named_like_a_socket(p)
            || SYSTEM.iter().any(|dir| inside(p, dir) || above(p, dir))
            || user_entries.iter().any(|entry| {
                let dir = comparable(&entry.to_string_lossy(), &[]);
                inside(p, &dir) || (above(p, &dir) && entry.exists())
            })
    })
}

/// The home folders of this user, as the environment names them.
fn homes() -> Vec<PathBuf> {
    ["HOME", "USERPROFILE"].iter().filter_map(|v| std::env::var_os(v)).map(PathBuf::from).filter(|p| p.as_os_str().len() > 1).collect()
}

/// The Windows folders Docker keeps its data in.
fn data_dirs() -> Vec<PathBuf> {
    ["PROGRAMDATA", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var_os(v)).map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn refused(spec: &str) -> bool {
        refused_in(&[spec.to_string()], &[], &[]).is_some()
    }

    #[test]
    fn a_socket_or_pipe_of_a_runtime_is_never_mounted_in_any_spelling() {
        for spec in [
            "/var/run/docker.sock:/var/run/docker.sock",
            "/run/docker.sock:/x:ro",
            "/var/run/docker.sock:/x",
            "/run/podman/podman.sock:/x",
            "/run/user/1000/podman/podman.sock:/x",
            "/run/containerd/containerd.sock:/x",
            "/var/run/crio/crio.sock:/x",
            "/run/k3s/containerd/containerd.sock:/x",
            "/mnt/wsl/docker-desktop/shared-sockets/host-services/docker.proxy.sock:/x",
            "/somewhere/else/my-docker.sock:/x",
            "//./pipe/docker_engine://./pipe/docker_engine",
            "\\\\.\\pipe\\docker_engine:\\\\.\\pipe\\docker_engine",
            "npipe:////./pipe/docker_engine:/x",
            "//./pipe/podman-machine-default:/x",
            "//?/pipe/anything:/x",
        ] {
            assert!(refused(spec), "{spec}");
        }
    }

    #[test]
    fn the_runtimes_folders_and_the_ones_above_them_are_refused() {
        for spec in ["/var/run/docker:/x", "/run/podman:/x", "/var/lib/docker:/x", "/var/lib/containers/storage:/x", "/mnt/wsl/docker-desktop-bind-mounts:/x", "/var/lib:/x", "/var/run:/x", "/run:/x", "/var:/x", "/mnt/wsl:/x", "/:/x", "/var/run/:/x"] {
            assert!(refused(spec), "{spec}");
        }
    }

    #[test]
    fn spellings_that_reach_the_same_place_are_refused_too() {
        for spec in ["/var/run/./docker.sock:/x", "/VAR/RUN/Docker.sock:/x", "/var//run//docker.sock:/x", "/tmp/../var/run/docker.sock:/x", "\\var\\run\\docker.sock:/x", "/var/run/docker.sock/:/x", "/Var/Lib/Docker:/x"] {
            assert!(refused(spec), "{spec}");
        }
    }

    #[test]
    fn an_ordinary_mount_is_left_alone() {
        for spec in ["data:/data", "docker-data:/data", "/home/me/repo:/work", "/tmp:/t", "/var/log:/log:ro", "/srv/app:/app", "C:\\Users\\me\\repo:/work", "/var/run/postgresql:/pg", "/opt/docker-notes:/n", "/usr/local/bin/dockerfile-lint:/l"] {
            assert!(!refused(spec), "{spec}");
        }
    }

    #[test]
    fn the_cli_session_folders_stay_refused_through_the_same_check() {
        assert_eq!(refused_in(&["data:/d".into(), "/tmp/cc-socks/abc:/x".into()], &[], &[]).as_deref(), Some("/tmp/cc-socks/abc"));
    }

    #[test]
    fn a_pipe_is_named_as_the_whole_entry_and_a_path_by_its_source() {
        assert_eq!(refused_in(&["data:/d".into(), "/var/run/docker.sock:/x".into()], &[], &[]).as_deref(), Some("/var/run/docker.sock"));
        assert_eq!(refused_in(&["npipe:////./pipe/docker_engine:/x".into()], &[], &[]).as_deref(), Some("npipe:////./pipe/docker_engine:/x"));
    }

    #[test]
    fn a_folder_holding_a_per_user_runtime_entry_is_refused_only_where_the_entry_is() {
        let with = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(with.path().join(".docker/run")).unwrap();
        let without = tempfile::tempdir().unwrap();
        let spec = |home: &Path, rel: &str| format!("{}:/x", home.join(rel).to_string_lossy().trim_end_matches(['/', '\\']));
        let homes = |h: &Path| vec![h.to_path_buf()];
        let check = |home: &Path, rel: &str| refused_in(&[spec(home, rel)], &homes(home), &[]).is_some();
        assert!(check(with.path(), ""), "a home folder that holds Docker Desktop's socket folder");
        assert!(check(with.path(), ".docker"), "the folder above it");
        assert!(check(with.path(), ".docker/run"), "and the folder itself");
        assert!(check(without.path(), ".colima/default"), "inside a runtime's folder, there or not");
        assert!(!check(without.path(), ""), "a home folder with none of them in it");
        assert!(!check(without.path(), ".docker"), "nor the folder above one that is not there");
        assert!(!check(without.path(), "projects"));
    }

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        let home = tempfile::tempdir().unwrap();
        assert!(refused_in(&["~/.colima:/x".into()], &[home.path().to_path_buf()], &[]).is_some());
        assert!(refused_in(&["~/projects:/x".into()], &[home.path().to_path_buf()], &[]).is_none());
    }

    #[test]
    fn windows_data_folders_of_docker_are_refused() {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("docker")).unwrap();
        let dir = data.path().to_string_lossy().to_string();
        assert!(refused_in(&[format!("{dir}/Docker/x:/x")], &[], &[data.path().to_path_buf()]).is_some());
        assert!(refused_in(&[format!("{dir}:/x")], &[], &[data.path().to_path_buf()]).is_some(), "the folder holding it");
    }

    #[test]
    fn the_workspace_folder_is_held_to_the_same_rule() {
        assert_eq!(refused_mount(&[], Some("/var/run/docker.sock")).as_deref(), Some("/var/run/docker.sock"));
        assert_eq!(refused_mount(&[], Some("/var/run")).as_deref(), Some("/var/run"));
        assert_eq!(refused_mount(&[], Some("//./pipe/docker_engine")).as_deref(), Some("//./pipe/docker_engine"));
        assert_eq!(refused_mount(&["data:/d".into()], Some("/home/me/repo")), None);
        assert_eq!(refused_mount(&[], Some("")), None);
        assert_eq!(refused_mount(&[], None), None);
    }

    #[test]
    fn a_definition_with_an_entry_a_container_may_not_have_is_refused_when_saved_and_stripped_when_it_comes_in() {
        let json = r#"["data:/d","/var/run/docker.sock:/var/run/docker.sock","/home/me/repo:/work"]"#;
        assert_eq!(refused_in_json(json).as_deref(), Some("/var/run/docker.sock"));
        assert_eq!(refused_in_json(r#"["data:/d"]"#), None);
        assert_eq!(refused_in_json("[]"), None);
        assert_eq!(refused_in_json("not json"), None);
        let kept: Vec<String> = serde_json::from_str(&without_refused(json)).unwrap();
        assert_eq!(kept, ["data:/d", "/home/me/repo:/work"]);
        let clean = r#"["data:/d"]"#;
        assert_eq!(without_refused(clean), clean, "a list with nothing to drop is not rewritten");
        assert_eq!(without_refused("[]"), "[]");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_socket_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("docker.sock");
        std::fs::write(&real, b"").unwrap();
        let link = dir.path().join("innocent");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(refused(&format!("{}:/x", link.to_string_lossy())));
    }
}
