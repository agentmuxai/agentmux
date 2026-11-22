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
/// (as in the streamed pane, which denies downloads), and the network the browser has: only the address proxy on `proxy_port` (every name is
/// looked up by the proxy), no QUIC and no DNS over HTTPS. The browser is started with none of these as switches.
pub fn wanted(proxy_port: u16) -> Value {
    json!({
        "URLBlocklist": BLOCKED,
        "DeveloperToolsAvailability": 2,
        "ExtensionInstallBlocklist": ["*"],
        "BrowserSignin": 0,
        "AllowFileSelectionDialogs": false,
        "DownloadRestrictions": 3,
        "ProxyMode": "fixed_servers",
        "ProxyServer": format!("socks5://127.0.0.1:{proxy_port}"),
        "ProxyBypassList": "<-loopback>",
        "QuicAllowed": false,
        "DnsOverHttpsMode": "off",
    })
}

/// The strings in a policy key that holds a list (none when the key is absent or not a list).
fn listed<'a>(v: &'a Value, key: &str) -> Vec<&'a str> {
    v[key].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// Does the policy in `text` hold every key `wanted` names: every address in `BLOCKED` blocked, the developer tools off, no extension may be
/// installed, no sign-in to the browser itself, no file dialog, no download, and the network only through the address proxy?
pub fn holds(text: &str, proxy_port: u16) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    let blocked = listed(&v, "URLBlocklist");
    BLOCKED.iter().all(|b| blocked.contains(b))
        && listed(&v, "ExtensionInstallBlocklist").contains(&"*")
        && v["BrowserSignin"] == json!(0)
        && v["DeveloperToolsAvailability"] == json!(2)
        && v["AllowFileSelectionDialogs"] == json!(false)
        && v["DownloadRestrictions"] == json!(3)
        && v["ProxyMode"] == json!("fixed_servers")
        && v["ProxyServer"] == json!(format!("socks5://127.0.0.1:{proxy_port}"))
        && v["ProxyBypassList"] == json!("<-loopback>")
        && v["QuicAllowed"] == json!(false)
        && v["DnsOverHttpsMode"] == json!("off")
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

/// The browser will start under the policy, or the words that say what is missing. The policy is the folder of the container (or dedicated
/// machine) this server runs in, which holds this one browser: it is never written by the server, and screen mode serves no streamed browser
/// next to it. `first` is true for the first screen session: no other browser may already be running in the container then.
pub fn ensure(binary: &Path, first: bool, proxy_port: u16) -> Result<(), String> {
    if sandboxed_install(binary) {
        return Err(format!(
            "screen mode does not run with {}: a snap or flatpak browser reads its policy from inside its own sandbox, where this check cannot see it. Use the Google Chrome or Chromium package",
            binary.display()
        ));
    }
    let path = policy_dir(binary).join(FILE_NAME);
    if !std::fs::read_to_string(&path).is_ok_and(|t| holds(&t, proxy_port)) {
        return Err(missing_words(&path, proxy_port));
    }
    if first && any_browser(&running_programs()) {
        return Err("screen mode needs the container to hold this one browser: another browser is already running in it".to_string());
    }
    Ok(())
}

/// What to install, in words: the file, and what it holds.
fn missing_words(path: &Path, proxy_port: u16) -> String {
    let text = serde_json::to_string_pretty(&wanted(proxy_port)).unwrap_or_default();
    format!(
        "screen mode needs the browser's managed policy file {} (it blocks local files, the browser's own pages, developer tools, extensions, the browser's own sign-in, file dialogs and downloads, and sets its network); it is missing or does not hold. Install it as the machine's administrator, with this content:
{text}",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wanted_policy_holds_and_blocks_what_the_streamed_pane_blocks() {
        let text = wanted(41234).to_string();
        assert!(holds(&text, 41234));
        assert!(!holds(&text, 41235), "the policy names the proxy's port");
        for scheme in ["file:", "chrome:", "chrome-untrusted:", "chrome-extension:", "devtools:", "view-source:"] {
            assert!(BLOCKED.iter().any(|b| b.starts_with(scheme)), "{scheme} is not blocked");
        }
    }

    #[test]
    fn a_policy_that_leaves_something_open_does_not_hold() {
        assert!(!holds("not json", 1));
        assert!(!holds("{}", 1));
        let mut v = wanted(1);
        v["DeveloperToolsAvailability"] = json!(1);
        assert!(!holds(&v.to_string(), 1));
        for (key, open) in [("AllowFileSelectionDialogs", json!(true)), ("DownloadRestrictions", json!(0))] {
            let mut v = wanted(1);
            v[key] = open;
            assert!(!holds(&v.to_string(), 1), "{key} left open still holds");
        }
        let mut v = wanted(1);
        v["URLBlocklist"] = json!(BLOCKED.iter().filter(|b| !b.starts_with("file")).collect::<Vec<_>>());
        assert!(!holds(&v.to_string(), 1));
    }

    #[test]
    fn a_policy_without_the_extension_block_or_with_browser_sign_in_on_does_not_hold() {
        for key in ["ExtensionInstallBlocklist", "BrowserSignin"] {
            let mut v = wanted(1);
            v.as_object_mut().unwrap().remove(key);
            assert!(!holds(&v.to_string(), 1), "{key} left out still holds");
        }
        for (key, open) in [
            ("ExtensionInstallBlocklist", json!([])),
            ("ExtensionInstallBlocklist", json!(["abcdefghijklmnop"])),
            ("ExtensionInstallBlocklist", json!("*")),
            ("BrowserSignin", json!(1)),
            ("BrowserSignin", json!(2)),
        ] {
            let mut v = wanted(1);
            v[key] = open.clone();
            assert!(!holds(&v.to_string(), 1), "{key} = {open} still holds");
        }
    }

    #[test]
    fn every_key_the_wanted_policy_names_is_checked() {
        // `wanted` and `holds` cannot drift apart: dropping any one key from the wanted policy must make it not hold.
        let all = wanted(1);
        for key in all.as_object().unwrap().keys() {
            let mut v = all.clone();
            v.as_object_mut().unwrap().remove(key);
            assert!(!holds(&v.to_string(), 1), "{key} is named by the policy but not checked");
        }
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
        assert!(ensure(&shim, true, 1).unwrap_err().contains("snap or flatpak"));
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
        let words = missing_words(&path, 41234);
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
