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

/// The browser will start under the policy, or the words that say what to install. Never writes: the file is system-wide.
pub fn ensure(binary: &Path) -> Result<(), String> {
    let path = policy_dir(binary).join(FILE_NAME);
    if std::fs::read_to_string(&path).is_ok_and(|t| holds(&t)) {
        return Ok(());
    }
    Err(missing_words(&path))
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
