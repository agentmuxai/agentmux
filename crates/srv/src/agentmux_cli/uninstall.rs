// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agentmux update` and `agentmux uninstall`: what a person can do about an install they made with the install
//! script. `update` says the one command that replaces this version; `uninstall` removes what the script put in
//! place (the service, the program folder and its links) and leaves the data folders unless it is told to remove
//! them. The shared folder (the person's agents, accounts and history, which the desktop app uses too) is never
//! touched. SPEC_HEADLESS_RENAME_AND_PER_PLATFORM section 4.

use std::path::{Path, PathBuf};

/// The command that installs the current release over this one, checked against the signed manifest.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://agentmux.ai/install.sh | sh -s -- --headless";

/// What `agentmux update` says. `channel` is the one the person asked about: its restart command names it unless it is the default.
pub fn update_text(channel: &str, default_channel: &str) -> String {
    let restart = if channel == default_channel { "agentmux restart".to_string() } else { format!("agentmux --channel {channel} restart") };
    format!(
        "AgentMux Headless is updated by running its install command again; it checks the new version against the signed manifest and puts it beside this one:\n  {INSTALL_COMMAND}\nThen run `{restart}` to start the new version."
    )
}

/// The channel folders under `channels_dir` other than `this`: the other instances that run from the same install.
pub fn other_channels(channels_dir: &Path, this: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(channels_dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name != this)
        .collect();
    names.sort();
    names
}

/// The folder the install script made (`~/.agentmux/headless`, or `~/.agentmux/server` from before the rename),
/// found from this program's own path: `<base>/<version>/bin/<program>` with a `current` link in `<base>`.
/// `None` for a program that is anywhere else, so nothing is removed that the script did not make.
pub fn install_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    let base = bin.parent()?.parent()?;
    let named = matches!(base.file_name()?.to_str()?, "headless" | "server");
    let in_agentmux = base.parent()?.file_name()? == ".agentmux";
    (named && in_agentmux && base.join("current").symlink_metadata().is_ok()).then(|| base.to_path_buf())
}

/// The links the script made in `local_bin` that lead into `base`.
pub fn links_into(local_bin: &Path, base: &Path) -> Vec<PathBuf> {
    let Ok(base) = base.canonicalize() else { return Vec::new() };
    ["agentmux", "agentmux-headless", "agentmux-server"]
        .iter()
        .map(|name| local_bin.join(name))
        .filter(|link| link.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()))
        .filter(|link| link.canonicalize().is_ok_and(|target| target.starts_with(&base)))
        .collect()
}

/// A channel's own folder (`.../channels/<channel>`), the only data `--remove-data` removes. `None` for any other path.
pub fn data_to_remove(instance_dir: &Path, channel: &str) -> Option<PathBuf> {
    let right = instance_dir.file_name()?.to_str()? == channel && instance_dir.parent()?.file_name()? == "channels";
    right.then(|| instance_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_names_the_one_command_and_the_restart_of_the_channel_asked_about() {
        let text = update_text("headless", "headless");
        assert!(text.contains("install.sh | sh -s -- --headless") && text.contains("`agentmux restart`"), "{text}");
        let other = update_text("work", "headless");
        assert!(other.contains("`agentmux --channel work restart`") && !other.contains("`agentmux restart`"), "{other}");
    }

    #[test]
    fn the_other_channels_are_the_folders_beside_this_one() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["headless", "work", "alpha"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("note.txt"), b"x").unwrap();
        assert_eq!(other_channels(dir.path(), "headless"), vec!["alpha".to_string(), "work".to_string()]);
        assert!(other_channels(&dir.path().join("missing"), "headless").is_empty());
    }

    #[test]
    fn an_install_is_found_only_by_the_layout_the_script_makes() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".agentmux").join("headless").join("0.60.30").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("agentmux");
        assert_eq!(install_of(&exe), None, "without a `current` link beside the versions it is not an install");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("0.60.30", home.path().join(".agentmux").join("headless").join("current")).unwrap();
            assert_eq!(install_of(&exe), Some(home.path().join(".agentmux").join("headless")));
        }
        // The old name of the folder counts; any other folder does not, whatever it holds.
        let elsewhere = home.path().join("tools").join("0.60.30").join("bin").join("agentmux");
        assert_eq!(install_of(&elsewhere), None);
        let odd = home.path().join(".agentmux").join("stuff").join("0.60.30").join("bin").join("agentmux");
        assert_eq!(install_of(&odd), None);
        assert_eq!(install_of(Path::new("/usr/bin/agentmux")), None);
    }

    #[cfg(unix)]
    #[test]
    fn only_links_into_the_install_are_taken() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join(".agentmux").join("headless");
        let bin = base.join("0.60.30").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("agentmux"), b"x").unwrap();
        let other = home.path().join("desktop-agentmux");
        std::fs::write(&other, b"y").unwrap();
        let local = home.path().join(".local").join("bin");
        std::fs::create_dir_all(&local).unwrap();
        std::os::unix::fs::symlink(bin.join("agentmux"), local.join("agentmux-headless")).unwrap();
        std::os::unix::fs::symlink(&other, local.join("agentmux")).unwrap();
        assert_eq!(links_into(&local, &base), vec![local.join("agentmux-headless")], "the desktop app's `agentmux` is not ours to remove");
    }

    #[test]
    fn data_removal_is_one_channels_own_folder_and_nothing_else() {
        let root = Path::new("/home/u/.agentmux");
        assert_eq!(data_to_remove(&root.join("channels").join("headless"), "headless"), Some(root.join("channels").join("headless")));
        assert_eq!(data_to_remove(&root.join("shared"), "headless"), None, "the shared folder is the person's agents and history");
        assert_eq!(data_to_remove(&root.join("channels"), "headless"), None);
        assert_eq!(data_to_remove(&root.join("channels").join("work"), "headless"), None);
        assert_eq!(data_to_remove(root, "headless"), None);
    }
}
