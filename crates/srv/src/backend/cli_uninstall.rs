// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Toolchain's Uninstall: remove AgentMux's own copy of one agent CLI,
//! `<shared>/cli/<provider>/<pin>/`, and nothing else. The person's own copy on
//! PATH, the CLI's sign-in state and every other pin are never touched
//! (docs/specs/SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md §4).
//!
//! Unlike a prune this is the person's own click, so disuse is not asked; the
//! guards that stay are the ones that protect a running agent: the install lock,
//! and no live pane or running process using the folder.

use agentmux_common::DataPaths;

use super::cli_install::{shared_cli_dir, try_lock_install, COMPLETE_MARKER};
use super::cli_prune::{dir_size, InUse};

#[derive(Debug, PartialEq, Eq)]
pub enum Uninstalled {
    Removed { bytes: u64 },
    /// There was no folder: nothing to do.
    NotInstalled,
}

/// Remove `<shared>/cli/<provider_id>/<pin>/`. `in_use` is what runs now
/// ([`super::cli_prune::gather_in_use`]); a folder one of its commands mentions
/// is left alone and the error says why.
pub fn uninstall(paths: &DataPaths, provider_id: &str, pin: &str, in_use: &InUse) -> Result<Uninstalled, String> {
    let dir = shared_cli_dir(paths, provider_id, pin).ok_or_else(|| format!("{provider_id}@{pin} is not a safe folder name"))?;
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Ok(_) => return Err(format!("{} is not a plain folder, so it was left alone", dir.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Uninstalled::NotInstalled),
        Err(e) => return Err(format!("could not look at {}: {e}", dir.display())),
    }
    let _lock = match try_lock_install(paths, provider_id, pin) {
        Ok(Some(guard)) => guard,
        Ok(None) => return Err(format!("{provider_id} is being installed right now; try again when it ends")),
        Err(e) => return Err(format!("could not take the install lock for {provider_id}: {e}")),
    };
    if in_use.mentions(&dir) {
        return Err(format!("a running agent uses {provider_id}; close it first"));
    }
    let bytes = dir_size(&dir);
    // The marker first: a folder half removed by a crash is already "not an install".
    let _ = std::fs::remove_file(dir.join(COMPLETE_MARKER));
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not remove {}: {e}", dir.display()))?;
    Ok(Uninstalled::Removed { bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn paths_in(root: &Path) -> DataPaths {
        let version_dir = root.join("channels").join("c").join("versions").join("1.0.0");
        DataPaths {
            home_dir: root.to_path_buf(),
            instance_dir: root.join("channels").join("c"),
            channel: "c".to_string(),
            data_dir: version_dir.join("data"),
            config_dir: root.join("channels").join("c").join("config"),
            logs_dir: version_dir.join("logs"),
            cef_cache_dir: version_dir.join("cef-cache"),
            agents_dir: root.join("channels").join("c").join("agents"),
            instance_runtime_dir: version_dir.join("runtime"),
            shared_dir: root.join("shared"),
            mode: agentmux_common::RuntimeMode::Portable,
        }
    }

    fn install(p: &DataPaths, provider: &str, pin: &str) -> std::path::PathBuf {
        let dir = p.shared_dir.join("cli").join(provider).join(pin);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join(provider), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join(COMPLETE_MARKER), pin).unwrap();
        dir
    }

    fn idle() -> InUse {
        InUse { live_commands: vec![], process_commands: Some(vec!["/usr/bin/vim".into()]) }
    }

    #[test]
    fn it_removes_that_pin_of_that_provider_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let p = paths_in(root.path());
        let goose = install(&p, "goose", "1.54.0");
        let other_pin = install(&p, "goose", "1.50.0");
        let other_provider = install(&p, "grok", "1.0.52");
        let sign_in = p.shared_dir.join("identities").join("x").join("goose");
        std::fs::create_dir_all(&sign_in).unwrap();

        assert_eq!(uninstall(&p, "goose", "1.54.0", &idle()), Ok(Uninstalled::Removed { bytes: 100 + "1.54.0".len() as u64 }));
        assert!(!goose.exists());
        assert!(other_pin.exists() && other_provider.exists() && sign_in.exists());
    }

    #[test]
    fn a_folder_that_is_not_there_is_nothing_to_do() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(uninstall(&paths_in(root.path()), "goose", "1.54.0", &idle()), Ok(Uninstalled::NotInstalled));
    }

    #[test]
    fn a_running_agent_keeps_its_cli() {
        let root = tempfile::tempdir().unwrap();
        let p = paths_in(root.path());
        let dir = install(&p, "goose", "1.54.0");
        let cmd = format!("{}{}bin{}goose acp", dir.display(), std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
        let in_use = InUse { live_commands: vec![cmd], process_commands: None };

        let err = uninstall(&p, "goose", "1.54.0", &in_use).unwrap_err();
        assert!(err.contains("close it first"), "{err}");
        assert!(dir.join(COMPLETE_MARKER).exists(), "an install left alone stays complete");
    }

    #[test]
    fn it_waits_for_an_install_in_progress() {
        let root = tempfile::tempdir().unwrap();
        let p = paths_in(root.path());
        let dir = install(&p, "goose", "1.54.0");
        let _held = try_lock_install(&p, "goose", "1.54.0").unwrap().expect("the lock is free");

        let err = uninstall(&p, "goose", "1.54.0", &idle()).unwrap_err();
        assert!(err.contains("being installed"), "{err}");
        assert!(dir.exists());
    }

    #[test]
    fn a_name_that_is_not_a_safe_folder_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let p = paths_in(root.path());
        for (provider, pin) in [("..", "1.0.0"), ("goose", "../x"), ("goose", ""), ("go/ose", "1.0.0")] {
            assert!(uninstall(&p, provider, pin, &idle()).is_err(), "{provider}@{pin}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_folder_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let p = paths_in(root.path());
        let real = root.path().join("elsewhere");
        std::fs::create_dir_all(&real).unwrap();
        let link = p.shared_dir.join("cli").join("goose").join("1.54.0");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(uninstall(&p, "goose", "1.54.0", &idle()).unwrap_err().contains("not a plain folder"));
        assert!(real.exists());
    }
}
