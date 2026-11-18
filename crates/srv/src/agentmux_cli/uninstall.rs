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

/// Is `path` inside `base`? By whole path components, so `~/.agentmux/headless-old/bin/x` is not inside
/// `~/.agentmux/headless`; case-insensitive on Windows, where the two spellings are the same folder.
pub fn is_under(path: &Path, base: &Path) -> bool {
    let key = |p: &Path| -> Vec<String> {
        p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).map(|c| if cfg!(windows) { c.to_lowercase() } else { c }).collect()
    };
    let (path, base) = (key(path), key(base));
    path.len() > base.len() && path.starts_with(&base)
}

/// Does `text` (a service definition) name a path inside `base`? Found by the base followed by a separator, so a folder
/// that only starts with the same letters does not match; case-insensitive on Windows. The base is also looked for as the
/// definition's own writer spells it (`%` doubled in a unit, `&` as `&amp;` in a plist or task), so an install path that
/// holds such a character is still found.
pub fn text_names_under(text: &str, base: &Path) -> bool {
    let fold = |s: &str| if cfg!(windows) { s.to_lowercase() } else { s.to_string() };
    let text = fold(text);
    let base = fold(&base.to_string_lossy());
    let base = base.trim_end_matches(['/', '\\']);
    !base.is_empty() && super::service::spellings(base).iter().any(|b| ['/', '\\'].iter().any(|sep| text.contains(&format!("{b}{sep}"))))
}

/// The folder the install script made (`~/.agentmux/headless`, or `~/.agentmux/server` from before the rename; on Windows
/// `%LOCALAPPDATA%\AgentMux\Headless`), found from this program's own path: `<base>/<version>/bin/<program>` with a
/// `current` link (on Windows a junction) in `<base>`. `None` for a program that is anywhere else, so nothing is removed
/// that the script did not make.
pub fn install_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    let base = bin.parent()?.parent()?;
    let (name, above) = (base.file_name()?.to_str()?, base.parent()?.file_name()?.to_str()?);
    let unix_layout = matches!(name, "headless" | "server") && above == ".agentmux";
    // Windows file names do not care about case.
    let windows_layout = cfg!(windows) && name.eq_ignore_ascii_case("Headless") && above.eq_ignore_ascii_case("AgentMux");
    ((unix_layout || windows_layout) && base.join("current").symlink_metadata().is_ok()).then(|| base.to_path_buf())
}

/// How much of the program folder is gone when `remove_program_folder` returns.
#[derive(Debug, PartialEq, Eq)]
pub enum Removal {
    Gone,
    /// Windows does not let a running program's own file be deleted: all else is removed, and a helper removes the rest
    /// as soon as this command ends.
    RestAfterExit,
    /// As above, but the helper could not be started: the person removes the folder by hand.
    RestIsYours,
}

/// Remove the program folder `base`. On Windows `running` (this program, inside `base`) stays until it ends: everything
/// else is removed now, and a hidden helper removes the folder as soon as this command has ended.
pub fn remove_program_folder(base: &Path, running: &Path) -> Result<Removal, String> {
    let failed = |e: std::io::Error| format!("cannot remove {}: {e}", super::service::plain_path(base).display());
    if !cfg!(windows) {
        return std::fs::remove_dir_all(base).map(|()| Removal::Gone).map_err(failed);
    }
    // The `current` junction is what makes this folder an install: it goes last, so a removal that stops half way (a
    // locked file, a virus scanner) can be run again. It is taken away itself, never what it leads to.
    let current = base.join("current");
    remove_all_but(base, &[running, &current]).map_err(failed)?;
    if current.symlink_metadata().is_ok() {
        std::fs::remove_dir(&current).or_else(|_| std::fs::remove_file(&current)).map_err(failed)?;
    }
    if !running.exists() {
        return std::fs::remove_dir_all(base).map(|()| Removal::Gone).map_err(failed);
    }
    Ok(if later_removal(base) { Removal::RestAfterExit } else { Removal::RestIsYours })
}

/// Everything under `dir` except the entries in `keep` and the folders that lead to them.
fn remove_all_but(dir: &Path, keep: &[&Path]) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if keep.contains(&path.as_path()) {
            continue;
        }
        let meta = path.symlink_metadata()?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            if keep.iter().any(|k| k.starts_with(&path)) {
                remove_all_but(&path, keep)?;
            } else {
                std::fs::remove_dir_all(&path)?;
            }
        } else {
            std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path))?;
        }
    }
    Ok(())
}

/// Start a hidden `cmd` that tries to remove `base` once a second (for up to two minutes) until it is gone, so this
/// program's own file goes as soon as it has ended, however long the rest of the command takes (`--remove-data` can be
/// slow). `false` when the path holds a character `cmd` would act on, so nothing is run on a guess.
#[cfg(windows)]
fn later_removal(base: &Path) -> bool {
    use agentmux_common::win32::NoWindow;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    let text = base.to_string_lossy();
    // `cmd` does not take the `\\?\` form of a path.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if text.chars().any(|c| matches!(c, '"' | '%' | '^' | '&' | '|' | '<' | '>' | '(' | ')' | '!')) {
        return false;
    }
    std::process::Command::new("cmd.exe")
        .raw_arg(format!(
            "/d /s /c \"for /l %n in (1,1,120) do (ping -n 2 127.0.0.1 >nul & rmdir /s /q \"{text}\" 2>nul & if not exist \"{text}\" exit)\""
        ))
        // Not the folder this command was run from: a helper that stands inside `base` would hold it and never remove it.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .no_window()
        .spawn()
        .is_ok()
}

#[cfg(not(windows))]
fn later_removal(_base: &Path) -> bool {
    false
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
    fn a_path_is_inside_an_install_by_whole_components() {
        let base = Path::new(if cfg!(windows) { r"C:\Users\a\AppData\Local\AgentMux\Headless" } else { "/home/a/.agentmux/headless" });
        assert!(is_under(&base.join("current").join("bin").join("agentmux"), base));
        assert!(is_under(&base.join("0.60.31").join("bin"), base));
        assert!(!is_under(base, base), "the folder itself is not inside itself");
        let sibling = base.with_file_name(format!("{}-old", base.file_name().unwrap().to_string_lossy()));
        assert!(!is_under(&sibling.join("bin").join("agentmux"), base), "a folder that only starts with the same name is another folder");
        assert!(!is_under(Path::new(if cfg!(windows) { r"D:\unpacked\bin\agentmux" } else { "/opt/unpacked/bin/agentmux" }), base));
        if cfg!(windows) {
            assert!(is_under(Path::new(r"c:\users\A\appdata\local\agentmux\headless\current\bin\agentmux.exe"), base), "Windows paths differ in case only");
        }
    }

    #[test]
    fn a_service_definition_names_the_install_only_by_a_path_inside_it() {
        let base = Path::new(if cfg!(windows) { r"C:\Users\a\AppData\Local\AgentMux\Headless" } else { "/home/a/.agentmux/headless" });
        let inside = if cfg!(windows) {
            r"<Command>C:\Users\a\AppData\Local\AgentMux\Headless\current\bin\agentmux.exe</Command>"
        } else {
            "ExecStart=/home/a/.agentmux/headless/current/bin/agentmux run"
        };
        let elsewhere = if cfg!(windows) {
            r"<Command>C:\Users\a\AppData\Local\AgentMux\Headless-old\bin\agentmux.exe</Command>"
        } else {
            "ExecStart=/home/a/.agentmux/headless-old/bin/agentmux run"
        };
        let unpacked = if cfg!(windows) { r"<Command>D:\unpacked\bin\agentmux.exe</Command>" } else { "ExecStart=/opt/unpacked/bin/agentmux run" };
        assert!(text_names_under(inside, base));
        assert!(!text_names_under(elsewhere, base), "a folder that only starts with the same name is another folder");
        assert!(!text_names_under(unpacked, base));
        assert!(!text_names_under("", base) && !text_names_under(inside, Path::new("")));
    }

    #[test]
    fn an_install_path_is_found_in_a_definition_as_its_writer_spells_it() {
        use super::super::service::{render_plist, render_task_xml, render_unit};
        let (log, home) = (Path::new("/l/x.log"), Path::new("/home/a"));
        let percent = Path::new("/home/a%b/.agentmux/headless");
        let unit = render_unit(&percent.join("current/bin/agentmux"), "stable", log, "/usr/bin", home);
        assert!(text_names_under(&unit, percent), "{unit}");
        let ampersand = Path::new("/Users/a&b/.agentmux/headless");
        let plist = render_plist(&ampersand.join("current/bin/agentmux"), "stable", log, "/usr/bin", home);
        assert!(text_names_under(&plist, ampersand), "{plist}");
        let task_base = Path::new(r"C:\Users\a&b\AppData\Local\AgentMux\Headless");
        let xml = render_task_xml(&task_base.join(r"current\bin\agentmux.exe"), "stable", None);
        assert!(text_names_under(&xml, task_base), "{xml}");
        assert!(!text_names_under(&unit, ampersand), "another install's path is still not this one");
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

    #[cfg(windows)]
    #[test]
    fn the_windows_install_is_the_headless_folder_under_agentmux_with_a_current_entry() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join("AppData").join("Local").join("AgentMux").join("Headless");
        let exe = base.join("0.60.30").join("bin").join("agentmux.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(install_of(&exe), None, "without a `current` entry it is not an install");
        std::fs::create_dir(base.join("current")).unwrap();
        assert_eq!(install_of(&exe), Some(base.clone()));
        // The names do not care about case; a folder of another name is not the install.
        let lower = home.path().join("l").join("agentmux").join("headless");
        std::fs::create_dir_all(lower.join("0.60.30").join("bin")).unwrap();
        std::fs::create_dir(lower.join("current")).unwrap();
        assert_eq!(install_of(&lower.join("0.60.30").join("bin").join("agentmux.exe")), Some(lower.clone()));
        let other = home.path().join("l").join("Tools").join("Headless");
        std::fs::create_dir_all(other.join("0.60.30").join("bin")).unwrap();
        std::fs::create_dir(other.join("current")).unwrap();
        assert_eq!(install_of(&other.join("0.60.30").join("bin").join("agentmux.exe")), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_program_folder_goes_but_for_the_running_program() {
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join("Headless");
        let old = base.join("0.60.29").join("bin");
        let new = base.join("0.60.30").join("bin");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::create_dir_all(base.join("0.60.30").join("web")).unwrap();
        std::fs::write(old.join("agentmux.exe"), b"o").unwrap();
        std::fs::write(new.join("agentmux.exe"), b"running").unwrap();
        std::fs::write(new.join("agentmux-srv.exe"), b"s").unwrap();
        std::fs::write(base.join("0.60.30").join("web").join("index.html"), b"w").unwrap();
        std::fs::create_dir(base.join("current")).unwrap();
        let running = new.join("agentmux.exe");
        let current = base.join("current");
        remove_all_but(&base, &[&running, &current]).unwrap();
        assert!(running.is_file(), "the running program stays");
        assert!(!old.exists() && !new.join("agentmux-srv.exe").exists() && !base.join("0.60.30").join("web").exists());
        assert!(current.exists(), "`current` is what a second run finds the install by: it goes last");
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
