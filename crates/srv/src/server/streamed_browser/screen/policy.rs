// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the browser may open. The streamed pane blocks local files and the browser's own pages through the debugging channel
//! (`pane.rs`, `BLOCKED_URLS`); in screen mode a person has the browser's own address bar, so the block is the browser's managed policy,
//! which the browser reads before it starts. Screen mode does not start without it: the policy is checked, written when this server
//! can, and refused in words when it cannot.

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

/// The policy: the blocked addresses, no developer tools, no extensions, no sign-in to the browser itself, and downloads that do not ask.
pub fn wanted() -> Value {
    json!({
        "URLBlocklist": BLOCKED,
        "DeveloperToolsAvailability": 2,
        "ExtensionInstallBlocklist": ["*"],
        "BrowserSignin": 0,
        "PromptForDownloadLocation": false,
    })
}

/// Does the policy in `text` block every address in `BLOCKED` and turn the developer tools off?
pub fn holds(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    let listed: Vec<&str> = v["URLBlocklist"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    BLOCKED.iter().all(|b| listed.contains(b)) && v["DeveloperToolsAvailability"] == json!(2)
}

/// The browser will start under the policy, or the words that say what to install. Writes it when it is missing or short.
pub fn ensure(binary: &Path) -> Result<(), String> {
    let path = policy_dir(binary).join(FILE_NAME);
    if std::fs::read_to_string(&path).is_ok_and(|t| holds(&t)) {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&wanted()).map_err(|e| e.to_string())?;
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, text.as_bytes()));
    written.map_err(|e| format!("screen mode needs the browser's policy file {} (it blocks local files, the browser's own pages and its developer tools), and this server cannot write it: {e}", path.display()))
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
        let mut v = wanted();
        v["URLBlocklist"] = json!(BLOCKED.iter().filter(|b| !b.starts_with("file")).collect::<Vec<_>>());
        assert!(!holds(&v.to_string()));
    }

    #[test]
    fn the_policy_folder_follows_the_kind_of_browser() {
        assert_eq!(policy_dir(Path::new("/usr/bin/chromium")), PathBuf::from("/etc/chromium/policies/managed"));
        assert_eq!(policy_dir(Path::new("/usr/bin/chromium-browser")), PathBuf::from("/etc/chromium/policies/managed"));
        assert_eq!(policy_dir(Path::new("/usr/bin/google-chrome")), PathBuf::from("/etc/opt/chrome/policies/managed"));
    }
}
