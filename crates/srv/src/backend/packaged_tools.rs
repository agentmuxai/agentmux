// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Store (MSIX) install keeps the app's tools (`agentmux-mcp`, `agentmux-bashwrap`
//! and the rest) inside the package folder under `WindowsApps`. A process that
//! doesn't have the package's identity can't run files from there (`EPERM` /
//! "Permission denied"), and an agent's shell and its MCP server are exactly
//! such processes: the agent lost its AgentMux tools and `Bash`.
//!
//! So a packaged install runs its tools from a copy: `~/.agentmux/tools/packaged/<package folder>/`,
//! made on first use and keyed by the package folder (which carries the version), so an
//! update stages a new one and never overwrites a copy a running agent uses. Any other
//! install (portable, setup.exe, `task dev`) is unchanged.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// Whether `path` is inside a Windows package install (`...\WindowsApps\<package>\...`).
pub(crate) fn is_package_path(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::Normal(n) if n.eq_ignore_ascii_case("WindowsApps")))
}

/// Where the copy of `source`'s tools goes: under the home's `.agentmux/tools/packaged/`, named for the
/// package folder that follows `WindowsApps` (so the version is in it).
pub(crate) fn staged_dir(source: &Path, home: &Path) -> Option<PathBuf> {
    let mut parts = source.components();
    parts.find(|c| matches!(c, Component::Normal(n) if n.eq_ignore_ascii_case("WindowsApps")))?;
    let package = match parts.next()? {
        Component::Normal(n) => n.to_string_lossy().into_owned(),
        _ => return None,
    };
    let name: String = package.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    Some(home.join(".agentmux").join("tools").join("packaged").join(name).join("bin"))
}

/// Copies `source`'s files into `dest`, skipping a file that is already there with the same size. A file
/// is copied under a temporary name and renamed, so a second srv starting at the same time never runs a
/// half-written program.
pub(crate) fn stage(source: &Path, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            stage(&from, &to)?;
            continue;
        }
        let len = entry.metadata()?.len();
        if std::fs::metadata(&to).map(|m| m.len() == len).unwrap_or(false) {
            continue;
        }
        let tmp = to.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::copy(&from, &tmp)?;
        if let Err(e) = std::fs::rename(&tmp, &to) {
            let _ = std::fs::remove_file(&tmp);
            // Another srv put the same file there first, and it may be running.
            if !std::fs::metadata(&to).map(|m| m.len() == len).unwrap_or(false) {
                return Err(e);
            }
        }
    }
    Ok(())
}

/// The folder to run the bundled tools from: `source` itself, except in a package install, where it is the
/// staged copy (made once per run; `source` again if the copy can't be made, which is logged).
pub(crate) fn runnable(source: PathBuf) -> PathBuf {
    static STAGED: OnceLock<PathBuf> = OnceLock::new();
    if !is_package_path(&source) || !source.is_dir() {
        return source;
    }
    STAGED
        .get_or_init(|| {
            let staged = dirs::home_dir().and_then(|home| staged_dir(&source, &home));
            match staged {
                Some(dest) => match stage(&source, &dest) {
                    Ok(()) => {
                        tracing::info!(from = %source.display(), to = %dest.display(), "packaged install: running the app's tools from a copy outside the package");
                        dest
                    }
                    Err(e) => {
                        tracing::warn!(from = %source.display(), to = %dest.display(), error = %e, "packaged install: couldn't copy the app's tools out of the package; agents may be unable to start them");
                        source.clone()
                    }
                },
                None => source.clone(),
            }
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_path_inside_windowsapps_is_a_package() {
        assert!(is_package_path(Path::new(r"C:\Program Files\WindowsApps\AgentMux.AgentMux_0.60.2.0_x64__abc\runtime\tools\bin")));
        assert!(is_package_path(Path::new("c:/program files/windowsapps/AgentMux.AgentMux_1_x64__abc/tools/bin")));
        assert!(!is_package_path(Path::new(r"C:\Users\a\AppData\Local\Programs\AgentMux\runtime\tools\bin")));
        assert!(!is_package_path(Path::new(r"C:\Users\a\Desktop\agentmux-portable\runtime\tools\bin")));
        assert!(!is_package_path(Path::new(r"C:\Users\a\WindowsAppsBackup\tools")));
    }

    #[test]
    fn the_copy_is_named_for_the_package_so_an_update_gets_its_own() {
        let home = Path::new("/home/a");
        let v1 = staged_dir(Path::new("C:/Program Files/WindowsApps/AgentMux.AgentMux_0.60.2.0_x64__abc/runtime/tools/bin"), home).unwrap();
        let v2 = staged_dir(Path::new("C:/Program Files/WindowsApps/AgentMux.AgentMux_0.60.3.0_x64__abc/runtime/tools/bin"), home).unwrap();
        assert_eq!(v1, home.join(".agentmux/tools/packaged/AgentMux.AgentMux_0.60.2.0_x64__abc/bin"));
        assert_ne!(v1, v2);
        assert_eq!(staged_dir(Path::new("C:/Users/a/AgentMux/tools/bin"), home), None);
    }

    #[test]
    fn staging_copies_files_and_folders_and_a_second_run_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("agentmux-mcp.exe"), b"mcp").unwrap();
        std::fs::write(src.join("sub").join("lib.dll"), b"lib").unwrap();

        stage(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("agentmux-mcp.exe")).unwrap(), b"mcp");
        assert_eq!(std::fs::read(dst.join("sub").join("lib.dll")).unwrap(), b"lib");

        // A copy that is already there (and may be running) is left alone.
        std::fs::write(dst.join("agentmux-mcp.exe"), b"xyz").unwrap();
        stage(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("agentmux-mcp.exe")).unwrap(), b"xyz");

        // A file of another size is replaced; no temporary file is left behind.
        std::fs::write(src.join("agentmux-mcp.exe"), b"longer").unwrap();
        stage(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("agentmux-mcp.exe")).unwrap(), b"longer");
        assert!(std::fs::read_dir(&dst).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn a_folder_that_isnt_in_a_package_is_used_as_it_is() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(runnable(tmp.path().to_path_buf()), tmp.path());
    }
}
