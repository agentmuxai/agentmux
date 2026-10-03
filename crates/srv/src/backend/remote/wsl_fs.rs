// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WSL folders in Hangar and the editor, through Windows' own share for a
//! distro's files, `\\wsl.localhost\<distro>\...` (P1 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! Windows reads and writes those paths with the ordinary file APIs, so the
//! panes need no new transport, only to know when a path is one: the rules
//! about where AgentMux may write are written for Windows paths, and a distro
//! has its own home and system folders, and its `/mnt/<letter>` is a Windows
//! drive. Everything here is string work on a path, so it is tested on every
//! platform.

/// The share's host. `wsl$` is the older name for the same share.
pub const SHARE_HOST: &str = "wsl.localhost";
const LEGACY_SHARE_HOST: &str = "wsl$";

/// A path inside a distro, as seen through the share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePath {
    pub distro: String,
    /// The path inside the distro: absolute, `/`-separated, no `.` or `..`.
    pub linux: String,
}

/// [`split_share`] where the share exists, on Windows; `None` elsewhere, where
/// `//wsl.localhost/...` is an ordinary path and the Windows rules for it are
/// the only ones. Every rule that treats a share path differently goes through
/// this.
pub fn share_of(path: &std::path::Path) -> Option<SharePath> {
    if cfg!(windows) {
        split_share(&path.to_string_lossy())
    } else {
        None
    }
}

/// The distro and Linux path of a path on the share: `\\wsl.localhost\D\...`,
/// `\\wsl$\D\...`, the `\\?\UNC\wsl.localhost\D\...` form `canonicalize`
/// returns, or any of those with `/` separators (the frontend's form). `None`
/// for any other path.
pub fn split_share(path: &str) -> Option<SharePath> {
    let rest = strip_unc_prefix(path)?;
    let mut parts = rest.split(['\\', '/']).filter(|p| !p.is_empty());
    let host = parts.next()?;
    if !host.eq_ignore_ascii_case(SHARE_HOST) && !host.eq_ignore_ascii_case(LEGACY_SHARE_HOST) {
        return None;
    }
    let distro = parts
        .next()
        .filter(|d| *d != "." && *d != "..")?
        .to_string();
    let mut linux: Vec<&str> = Vec::new();
    for part in parts {
        match part {
            "." => {}
            ".." => {
                linux.pop();
            }
            p => linux.push(p),
        }
    }
    Some(SharePath {
        distro,
        linux: format!("/{}", linux.join("/")),
    })
}

fn strip_unc_prefix(path: &str) -> Option<&str> {
    [r"\\?\UNC\", "//?/UNC/", r"\\", "//"]
        .into_iter()
        .find(|prefix| {
            path.get(..prefix.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
        })
        .map(|prefix| &path[prefix.len()..])
}

/// The share path for `linux` in `distro`: `\\wsl.localhost\<distro>\a\b`.
pub fn share_path(distro: &str, linux: &str) -> String {
    let mut out = format!(r"\\{SHARE_HOST}\{distro}\");
    out.push_str(&linux.trim_start_matches('/').replace('/', "\\"));
    out
}

/// A distro's own system folders: the same list AgentMux protects on a Linux
/// machine, plus `/mnt` (where WSL mounts the Windows drives and its own
/// helpers), `/init` (WSL's own init) and `/run`. `/` itself counts.
const SYSTEM_DIRS: &[&str] = &[
    "/usr", "/bin", "/sbin", "/etc", "/var", "/boot", "/lib", "/lib32", "/lib64", "/proc", "/sys",
    "/dev", "/mnt", "/init", "/run",
];

/// Whether `linux` is, or is inside, one of a distro's system folders.
pub fn is_system_path(linux: &str) -> bool {
    linux == "/"
        || SYSTEM_DIRS
            .iter()
            .any(|d| linux == *d || linux.starts_with(&format!("{d}/")))
}

/// The Windows path a distro's `/mnt/<letter>/...` is: WSL mounts each drive
/// there, so changing it changes the Windows file, and the Windows rules apply.
pub fn drive_path(linux: &str) -> Option<String> {
    let rest = linux.strip_prefix("/mnt/")?;
    let (letter, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let mut chars = letter.chars();
    let c = chars.next().filter(|c| c.is_ascii_alphabetic())?;
    if chars.next().is_some() {
        return None;
    }
    Some(format!(
        "{}:\\{}",
        c.to_ascii_uppercase(),
        tail.replace('/', "\\")
    ))
}

/// Whether `linux` is a distro user's home folder: `/home/<user>` or `/root`.
pub fn is_home(linux: &str) -> bool {
    linux == "/root"
        || linux
            .strip_prefix("/home/")
            .is_some_and(|u| !u.is_empty() && !u.contains('/'))
}

/// Whether `linux` is a distro user's home folder or inside one: the WSL side
/// of the editor's "only under your home folder" rule for writes.
pub fn in_home(linux: &str) -> bool {
    is_home(linux)
        || linux.starts_with("/root/")
        || linux
            .strip_prefix("/home/")
            .is_some_and(|u| u.split_once('/').is_some_and(|(user, _)| !user.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(distro: &str, linux: &str) -> Option<SharePath> {
        Some(SharePath {
            distro: distro.to_string(),
            linux: linux.to_string(),
        })
    }

    #[test]
    fn every_spelling_of_a_share_path_splits_the_same_way() {
        for path in [
            r"\\wsl.localhost\Ubuntu\home\u\proj",
            r"\\WSL.LOCALHOST\Ubuntu\home\u\proj\",
            r"\\wsl$\Ubuntu\home\u\proj",
            r"\\?\UNC\wsl.localhost\Ubuntu\home\u\proj",
            "//wsl.localhost/Ubuntu/home/u/proj",
            r"\\wsl.localhost\Ubuntu\home\u\.\x\..\proj",
        ] {
            assert_eq!(split_share(path), sp("Ubuntu", "/home/u/proj"), "{path}");
        }
        assert_eq!(split_share(r"\\wsl.localhost\Ubuntu"), sp("Ubuntu", "/"));
        assert_eq!(
            split_share(r"\\wsl.localhost\Ubuntu\..\..\x"),
            sp("Ubuntu", "/x")
        );
    }

    #[test]
    fn other_paths_are_not_share_paths() {
        for path in [
            r"C:\Users\u",
            r"\\?\C:\Users\u",
            r"\\server\share\x",
            r"\\wsl.localhost",
            r"\\wsl.localhost\..\x",
            "/home/u",
            "",
        ] {
            assert_eq!(split_share(path), None, "{path}");
        }
    }

    /// Off Windows a share-looking path is an ordinary one: no WSL rule applies.
    #[test]
    fn a_share_path_is_one_only_on_windows() {
        let p = std::path::Path::new("//wsl.localhost/Ubuntu/home/u/x");
        assert_eq!(share_of(p).is_some(), cfg!(windows));
    }

    #[test]
    fn share_paths_round_trip() {
        assert_eq!(share_path("Ubuntu", "/"), r"\\wsl.localhost\Ubuntu\");
        let p = share_path("Ubuntu", "/home/u/proj");
        assert_eq!(p, r"\\wsl.localhost\Ubuntu\home\u\proj");
        assert_eq!(split_share(&p), sp("Ubuntu", "/home/u/proj"));
    }

    #[test]
    fn a_distro_has_system_folders_and_homes() {
        for p in [
            "/",
            "/usr",
            "/usr/bin/ls",
            "/etc/passwd",
            "/mnt",
            "/mnt/wsl",
            "/init",
        ] {
            assert!(is_system_path(p), "{p}");
        }
        for p in [
            "/home/u",
            "/root/x",
            "/srv/app",
            "/usrlocal",
            "/tmp/x",
            "/opt/tool",
        ] {
            assert!(!is_system_path(p), "{p}");
        }
        assert!(is_home("/home/u") && is_home("/root"));
        assert!(!is_home("/home") && !is_home("/home/u/x"));
        assert!(in_home("/home/u") && in_home("/home/u/proj/a.rs") && in_home("/root/x"));
        assert!(
            !in_home("/home")
                && !in_home("/srv/app")
                && !in_home("/rootx")
                && !in_home("/mnt/c/Users/u")
        );
    }

    #[test]
    fn a_drive_under_mnt_is_its_windows_path() {
        assert_eq!(drive_path("/mnt/c"), Some(r"C:\".to_string()));
        assert_eq!(
            drive_path("/mnt/c/Windows/System32"),
            Some(r"C:\Windows\System32".to_string())
        );
        assert_eq!(drive_path("/mnt/d/x"), Some(r"D:\x".to_string()));
        assert_eq!(drive_path("/mnt/wsl"), None);
        assert_eq!(drive_path("/mnt"), None);
        assert_eq!(drive_path("/home/u"), None);
    }
}
