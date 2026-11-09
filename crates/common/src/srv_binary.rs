// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the srv binary may be, for the launcher and the CEF host. Each anchors its own folders; the file names and
//! the order within a folder are the same.

use std::path::{Path, PathBuf};

/// The release these binaries belong to (the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `.exe` on Windows, nothing elsewhere.
pub const EXE_SUFFIX: &str = if cfg!(target_os = "windows") { ".exe" } else { "" };

/// The platform and CPU in a portable build's file names.
fn os_arch() -> (&'static str, &'static str) {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    if cfg!(target_os = "macos") {
        ("darwin", arch)
    } else if cfg!(target_os = "linux") {
        ("linux", arch)
    } else {
        ("windows", arch)
    }
}

/// `<name>-<version>-<os>.<arch><suffix>`, the name a portable build gives the binary.
pub fn versioned_name(name: &str, exe_suffix: &str) -> String {
    let (os, arch) = os_arch();
    format!("{name}-{VERSION}-{os}.{arch}{exe_suffix}")
}

/// Every place the binary may be, in search order: in each of `dirs` the versioned name, then the plain one
/// (a cargo build's output).
pub fn candidates(dirs: &[&Path], name: &str, exe_suffix: &str) -> Vec<PathBuf> {
    dirs.iter()
        .flat_map(|dir| [dir.join(versioned_name(name, exe_suffix)), dir.join(format!("{name}{exe_suffix}"))])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_dir_gives_the_versioned_name_then_the_plain_one() {
        let found = candidates(&[Path::new("a"), Path::new("b")], "agentmux-srv", ".exe");
        assert_eq!(found.len(), 4);
        assert_eq!(found[0], Path::new("a").join(versioned_name("agentmux-srv", ".exe")));
        assert_eq!(found[1], Path::new("a").join("agentmux-srv.exe"));
        assert_eq!(found[2], Path::new("b").join(versioned_name("agentmux-srv", ".exe")));
        assert_eq!(found[3], Path::new("b").join("agentmux-srv.exe"));
    }

    #[test]
    fn the_versioned_name_carries_version_os_and_arch() {
        let n = versioned_name("agentmux-srv", "");
        assert!(n.starts_with(&format!("agentmux-srv-{VERSION}-")), "{n}");
        assert!(n.contains('.'), "{n}");
    }
}
