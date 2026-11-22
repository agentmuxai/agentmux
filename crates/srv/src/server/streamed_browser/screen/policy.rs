// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the browser may open. The streamed pane blocks local files and the browser's own pages through the debugging channel
//! (`pane.rs`, `BLOCKED_URLS`); in screen mode a person has the browser's own address bar, so the block is the browser's managed policy,
//! which the browser reads before it starts. Screen mode does not start without it. This server only checks that the policy is there: the file
//! is system-wide (every browser of that kind on the machine reads it), so whoever runs the machine or builds its image installs it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Address patterns the browser never opens: local files, its own pages and tools, and an address a page could make from a string.
pub const BLOCKED: &[&str] = &[
    "file://*",
    "chrome://*",
    "chrome-untrusted://*",
    "chrome-extension://*",
    "chrome-search://*",
    "chrome-error://*",
    "devtools://*",
    "view-source:*",
    "javascript:*",
    "ftp://*",
];

const FILE_NAME: &str = "agentmux-screen-mode.json";

/// The folder the browser reads managed policy from, by the kind of browser it is.
pub fn policy_dir(binary: &Path) -> PathBuf {
    let name = binary.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    if name.contains("chromium") {
        PathBuf::from("/etc/chromium/policies/managed")
    } else {
        PathBuf::from("/etc/opt/chrome/policies/managed")
    }
}

/// The policy: the blocked addresses, no developer tools, no extensions, no sign-in to the browser itself, no file dialogs and no downloads
/// (as in the streamed pane, which denies downloads).
pub fn wanted() -> Value {
    json!({
        "URLBlocklist": BLOCKED,
        "DeveloperToolsAvailability": 2,
        "ExtensionInstallBlocklist": ["*"],
        "BrowserSignin": 0,
        "AllowFileSelectionDialogs": false,
        "DownloadRestrictions": 3,
    })
}

/// Does the policy in `text` block every address in `BLOCKED`, turn the developer tools off, and allow no file dialog and no download?
pub fn holds(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    let listed: Vec<&str> = v["URLBlocklist"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    BLOCKED.iter().all(|b| listed.contains(b))
        && v["DeveloperToolsAvailability"] == json!(2)
        && v["AllowFileSelectionDialogs"] == json!(false)
        && v["DownloadRestrictions"] == json!(3)
}

/// Is the browser a snap or a flatpak, or the shim that starts one? Such a browser reads its policy from inside its own sandbox, not from the
/// folder checked here, so a file there would pass the check and be ignored by the browser.
pub fn sandboxed_install(binary: &Path) -> bool {
    use std::io::Read;
    let real = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let named = |p: &Path| {
        let s = p.to_string_lossy();
        s.contains("/snap/") || s.contains("/flatpak/") || p.file_name().is_some_and(|n| n == "snap" || n == "flatpak")
    };
    if named(binary) || named(&real) {
        return true;
    }
    // A small script that starts a snap (the deb that only hands over to the snap on Ubuntu).
    let mut head = [0u8; 4096];
    let n = std::fs::File::open(&real).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let text = String::from_utf8_lossy(&head[..n]);
    text.starts_with("#!") && (text.contains("/snap/") || text.contains("snap run") || text.contains("flatpak run"))
}

/// Where the browser's policy comes from: a folder made for this one instance (a mount namespace, the shared-host form), or the folder of a
/// container that holds only this browser (the task-scoped form, chosen by `FORM_ENV=task`). Never a machine-wide file on a machine that also
/// runs a streamed browser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Namespace,
    Task,
}

/// The setting that says this server is a hosted task that holds one screen-mode browser and no streamed browser.
pub const FORM_ENV: &str = "AGENTMUX_STREAMED_BROWSER_SCREEN_POLICY";

pub fn form() -> Form {
    form_of(std::env::var(FORM_ENV).ok().as_deref())
}

fn form_of(setting: Option<&str>) -> Form {
    if setting == Some("task") {
        Form::Task
    } else {
        Form::Namespace
    }
}

/// Write this instance's policy into `dir` (made if it is not there): the folder the namespace puts where the browser reads its policy.
pub fn write_instance(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the browser's policy folder: {e}"))?;
    let text = serde_json::to_string_pretty(&wanted()).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(FILE_NAME), text).map_err(|e| format!("cannot write the browser's policy: {e}"))
}

/// Whether any of these program names is a Chrome or Chromium (a process's `comm`).
fn any_browser(names: &[String]) -> bool {
    names.iter().any(|n| n.starts_with("chrome") || n.starts_with("chromium"))
}

/// The names of the programs running in this container (each process's `comm`).
fn running_programs() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .map(|s| s.trim().to_string())
        .collect()
}

/// The browser will start under the policy for its form, or the words that say what is missing. Never writes a machine-wide file.
/// Namespace form: bubblewrap must be there. Task form: the container's policy file must hold, and no browser may already be running in
/// the container (a streamed browser sharing it would be bound by the same file).
pub fn ensure(binary: &Path) -> Result<(), String> {
    if sandboxed_install(binary) {
        return Err(format!(
            "screen mode does not run with {}: a snap or flatpak browser reads its policy from inside its own sandbox, where this check cannot see it. Use the Google Chrome or Chromium package",
            binary.display()
        ));
    }
    match form() {
        Form::Namespace => which::which(super::confine::PROGRAM).map(|_| ()).map_err(|_| {
            "screen mode needs bubblewrap (the `bwrap` program) to give each browser its own policy folder; install it (for example `apt install bubblewrap`). It does not write a machine-wide policy file".to_string()
        }),
        Form::Task => {
            let path = policy_dir(binary).join(FILE_NAME);
            if !std::fs::read_to_string(&path).is_ok_and(|t| holds(&t)) {
                return Err(missing_words(&path));
            }
            if any_browser(&running_programs()) {
                return Err("screen mode in a task-scoped container needs the container to hold this one browser: another browser is already running in it".to_string());
            }
            Ok(())
        }
    }
}

/// What to install, in words: the file, and what it holds.
fn missing_words(path: &Path) -> String {
    let text = serde_json::to_string_pretty(&wanted()).unwrap_or_default();
    format!(
        "screen mode needs the browser's managed policy file {} (it blocks local files, the browser's own pages, developer tools, file dialogs and downloads); it is missing or does not hold. Install it as the machine's administrator, with this content:
{text}",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wanted_policy_holds_and_blocks_what_the_streamed_pane_blocks() {
        let text = wanted().to_string();
        assert!(holds(&text));
        for scheme in ["file:", "chrome:", "chrome-untrusted:", "chrome-extension:", "devtools:", "view-source:"] {
            assert!(BLOCKED.iter().any(|b| b.starts_with(scheme)), "{scheme} is not blocked");
        }
    }

    #[test]
    fn a_policy_that_leaves_something_open_does_not_hold() {
        assert!(!holds("not json"));
        assert!(!holds("{}"));
        let mut v = wanted();
        v["DeveloperToolsAvailability"] = json!(1);
        assert!(!holds(&v.to_string()));
        for (key, open) in [("AllowFileSelectionDialogs", json!(true)), ("DownloadRestrictions", json!(0))] {
            let mut v = wanted();
            v[key] = open;
            assert!(!holds(&v.to_string()), "{key} left open still holds");
        }
        let mut v = wanted();
        v["URLBlocklist"] = json!(BLOCKED.iter().filter(|b| !b.starts_with("file")).collect::<Vec<_>>());
        assert!(!holds(&v.to_string()));
    }

    #[test]
    fn a_snap_or_flatpak_browser_is_refused_and_a_plain_one_is_not() {
        assert!(sandboxed_install(Path::new("/snap/bin/chromium")));
        assert!(sandboxed_install(Path::new("/var/lib/flatpak/app/org.chromium.Chromium/x")));
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("chromium-browser");
        std::fs::write(&shim, "#!/bin/sh
exec /snap/bin/chromium \"$@\"
").unwrap();
        assert!(sandboxed_install(&shim), "a script that starts the snap");
        let plain = dir.path().join("google-chrome");
        std::fs::write(&plain, "#!/bin/sh
exec /opt/google/chrome/chrome \"$@\"
").unwrap();
        assert!(!sandboxed_install(&plain));
        assert!(ensure(&shim).unwrap_err().contains("snap or flatpak"));
    }

    #[test]
    fn the_form_is_the_shared_host_one_unless_the_task_form_is_asked_for() {
        assert_eq!(form_of(None), Form::Namespace);
        assert_eq!(form_of(Some("")), Form::Namespace);
        assert_eq!(form_of(Some("task")), Form::Task);
    }

    #[test]
    fn the_instances_policy_is_written_and_holds() {
        let dir = tempfile::tempdir().unwrap();
        write_instance(&dir.path().join("managed")).unwrap();
        let text = std::fs::read_to_string(dir.path().join("managed").join(FILE_NAME)).unwrap();
        assert!(holds(&text));
    }

    #[test]
    fn another_browser_in_the_container_is_seen_by_its_program_name() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(any_browser(&names(&["srv", "chrome"])));
        assert!(any_browser(&names(&["chromium"])));
        assert!(!any_browser(&names(&["srv", "Xvfb", "openbox"])));
    }

    #[test]
    fn a_missing_policy_is_refused_in_words_and_nothing_is_written() {
        let dir = std::env::temp_dir().join("agentmux-policy-check-only");
        let path = dir.join(FILE_NAME);
        let words = missing_words(&path);
        assert!(words.contains("DownloadRestrictions") && words.contains("URLBlocklist") && words.contains(FILE_NAME), "{words}");
        assert!(!path.exists(), "checking must not write the file");
    }

    #[test]
    fn the_policy_folder_follows_the_kind_of_browser() {
        assert_eq!(policy_dir(Path::new("/usr/bin/chromium")), PathBuf::from("/etc/chromium/policies/managed"));
        assert_eq!(policy_dir(Path::new("/usr/bin/chromium-browser")), PathBuf::from("/etc/chromium/policies/managed"));
        assert_eq!(policy_dir(Path::new("/usr/bin/google-chrome")), PathBuf::from("/etc/opt/chrome/policies/managed"));
    }
}
