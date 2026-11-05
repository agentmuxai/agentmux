// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A LAN peer in words: how it is installed and whether it runs agents. A host can run two
//! installs (the Microsoft Store copy and a setup.exe one), and a peer that is an install with no
//! agents answers every lookup of an agent with "not found"; saying which peer it was shows the
//! real cause.

use super::discovery::LanInstance;

/// A peer's `os` and `install` from its mDNS TXT record: self-reported, so only a plain token is kept.
pub(crate) fn os_and_install(info: &mdns_sd::ServiceInfo) -> (String, String) {
    let token = |key: &str| info.get_property_val_str(key).and_then(crate::backend::host_os::sanitize_os).unwrap_or_default();
    (token("os"), token("install"))
}

/// A re-fired resolution often carries an empty TXT record: apply `new` only when it says something,
/// so a stale re-fire never erases an install already known.
pub(crate) fn keep_known(known: &mut String, new: String) {
    if !new.is_empty() {
        *known = new;
    }
}

/// How an install is said in a message ("Microsoft Store"); empty when the peer didn't say or it is unknown.
pub(crate) fn install_words(install: &str) -> &'static str {
    match install {
        "windows-store" => "Microsoft Store",
        "windows-setup" => "installed",
        "windows-portable" | "linux-portable" => "portable",
        "macos-app" => "app",
        "linux-appimage" => "AppImage",
        "linux-package" => "package",
        _ => "",
    }
}

/// A peer in words, so the real cause of a missing agent shows: "Area54/stable (Microsoft Store, no agents)".
/// "No agents" is said only of a peer that was asked and runs nobody.
pub(crate) fn describe_peer(p: &LanInstance) -> String {
    let host = p.hostname.trim();
    let name = if p.channel.is_empty() { host.to_string() } else { format!("{host}/{}", p.channel) };
    let mut notes: Vec<&str> = Vec::new();
    let how = install_words(&p.install);
    if !how.is_empty() {
        notes.push(how);
    }
    if p.last_polled_ok > 0 && p.agents.is_empty() {
        notes.push("no agents");
    }
    if notes.is_empty() {
        name
    } else {
        format!("{name} ({})", notes.join(", "))
    }
}
