// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

fn main() {
    // Only the Windows resources below depend on anything outside the Rust
    // sources, and nothing here embeds a git hash, so narrowing Cargo's rerun
    // default to these two paths is safe. Listing the icon means an icon edit
    // actually re-embeds it.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../cef/resources/win/agentmux.ico");

    // Windows: icon + version info. Task Manager shows FileDescription as the
    // process name and the icon next to it; without an icon resource it shows
    // the generic one (docs/reports/REPORT_PROCESS_ICONS_NAMES_AND_LABELS_2026_10_10.md).
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux Server v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux Corp");
        res.set("InternalName", "agentmux-srv");
        let icon_path = std::path::Path::new("../cef/resources/win/agentmux.ico");
        if icon_path.exists() {
            res.set_icon(icon_path.to_str().unwrap());
        }
        res.compile().expect("winres compile failed");
    }
}
