// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What kind of build this binary is: a published release, a local package
//! build, or a source-tree dev build.
//!
//! It is a fact of the binary, never of the data channel it runs on. An
//! in-app upgrade keeps the data channel of the build it replaced
//! (`runtime_mode::recorded_channel`), so a published release that a local
//! build upgraded to runs on a `local-…` channel for good. Anything that
//! says or decides "this is a local build" from the channel name is wrong
//! there. Ask this instead, and show the channel only as where the data is.

use std::path::Path;

/// The kind of build a binary is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    /// Built by the release pipeline: what the website, the stores and the
    /// in-app upgrade hand out.
    Release,
    /// Built on someone's machine by the packaging scripts (`task package`).
    Local,
    /// Run from a source tree (`task dev`, `cargo run`).
    Dev,
}

impl BuildKind {
    /// The token sent to peers and shown in listings: `release`, `local` or `dev`.
    pub fn label(self) -> &'static str {
        match self {
            BuildKind::Release => "release",
            BuildKind::Local => "local",
            BuildKind::Dev => "dev",
        }
    }

    /// The kind a peer's token names; `None` for anything else, including the
    /// empty string of a peer from before the kind was advertised.
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "release" => Some(BuildKind::Release),
            "local" => Some(BuildKind::Local),
            "dev" => Some(BuildKind::Dev),
            _ => None,
        }
    }
}

/// The kind of a build from what its binary carries:
/// - `dev`: it runs from a source tree's output folder;
/// - `portable_label`: the label in a portable folder's
///   `agentmux-portable.marker` (`runtime_mode::portable_build_label`). A
///   release portable's label is its plain version (`0.60.22`); a local one's
///   carries the commit and a build stamp (`0.60.22+g2a5424fe2.…`);
/// - `made_for_channel`: the channel the packaging script compiled in as the
///   build's default. The release pipeline compiles in `stable`; a local
///   package build gets a `local-…` channel of its own. This is a constant of
///   the binary: an upgrade's recorded channel does not change it.
pub fn build_kind_of(dev: bool, portable_label: Option<&str>, made_for_channel: &str) -> BuildKind {
    if dev {
        BuildKind::Dev
    } else if portable_label.is_some_and(|l| !l.trim().is_empty() && !is_plain_version(l.trim())) || made_for_channel != "stable" {
        BuildKind::Local
    } else {
        BuildKind::Release
    }
}

/// `x.y.z`, three numbers and nothing else: what a release calls itself.
fn is_plain_version(label: &str) -> bool {
    let mut parts = label.split('.');
    let three = (0..3).all(|_| parts.next().is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())));
    three && parts.next().is_none()
}

/// The kind of the build whose binary sits in `exe_dir`.
pub fn build_kind_in(exe_dir: &Path) -> BuildKind {
    let dev = crate::runtime_mode::exe_dir_is_dev(exe_dir);
    let label = crate::runtime_mode::portable_build_label(exe_dir);
    build_kind_of(dev, label.as_deref(), crate::data_paths::build_channel_default())
}

/// The kind of the running binary. Read once: the binary does not change
/// under a running process.
pub fn build_kind() -> BuildKind {
    static KIND: std::sync::OnceLock<BuildKind> = std::sync::OnceLock::new();
    *KIND.get_or_init(|| {
        let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        match dir {
            Some(dir) => build_kind_in(&dir),
            // No path to judge by: only the compiled-in channel is left.
            None => build_kind_of(false, None, crate::data_paths::build_channel_default()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_is_a_release_whatever_channel_it_runs_on() {
        // The function takes no running channel at all: a release that kept a
        // local build's data channel through an upgrade is still a release.
        assert_eq!(build_kind_of(false, None, "stable"), BuildKind::Release);
    }

    #[test]
    fn a_local_package_build_is_local_by_its_marker_or_its_compiled_channel() {
        assert_eq!(build_kind_of(false, Some("0.60.22+g2a5424fe2.20261110T070000.1"), "local-main-b28b7a-0000000a"), BuildKind::Local);
        // An AppImage or a package has no marker file; the compiled-in channel still says so.
        assert_eq!(build_kind_of(false, None, "local-main-b28b7a-0000000a"), BuildKind::Local);
        // A stamped label says local even when `stable` was compiled in.
        assert_eq!(build_kind_of(false, Some("0.60.22+gabc.1"), "stable"), BuildKind::Local);
        assert_eq!(build_kind_of(false, Some("  "), "stable"), BuildKind::Release);
    }

    #[test]
    fn a_release_portable_carries_a_marker_with_its_plain_version() {
        assert_eq!(build_kind_of(false, Some("0.60.22"), "stable"), BuildKind::Release);
        for not_plain in ["0.60", "0.60.22.1", "0.60.x", "v0.60.22", "0.60.22-rc1"] {
            assert!(!is_plain_version(not_plain), "{not_plain}");
        }
    }

    #[test]
    fn a_source_tree_build_is_dev_first() {
        assert_eq!(build_kind_of(true, None, "stable"), BuildKind::Dev);
        assert_eq!(build_kind_of(true, Some("x"), "local-x-000000-00000000"), BuildKind::Dev);
    }

    #[test]
    fn labels_round_trip_and_unknown_tokens_are_refused() {
        for kind in [BuildKind::Release, BuildKind::Local, BuildKind::Dev] {
            assert_eq!(BuildKind::from_label(kind.label()), Some(kind));
        }
        for other in ["", "stable", "Release", "local-main-b28b7a-0000000a"] {
            assert_eq!(BuildKind::from_label(other), None, "{other:?}");
        }
    }
}
