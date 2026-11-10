// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

fn main() {
    // Nothing here embeds a git hash, so narrowing Cargo's rerun default to
    // these two paths is safe; listing the icon means an icon edit re-embeds it.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../cef/resources/win/agentmux.ico");

    // Windows: icon + version info, so Task Manager shows "AgentMux Shell
    // Wrapper" with the AgentMux icon instead of a bare file name and the
    // generic icon. Costs nothing at run time: the resource section isn't read
    // to start the process, which matters for a binary that runs once per
    // shell command (docs/reports/REPORT_PROCESS_ICONS_NAMES_AND_LABELS_2026_10_10.md).
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux Shell Wrapper v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux Corp");
        res.set("InternalName", "agentmux-bashwrap");
        let icon_path = std::path::Path::new("../cef/resources/win/agentmux.ico");
        if icon_path.exists() {
            res.set_icon(icon_path.to_str().unwrap());
        }
        res.compile().expect("winres compile failed");
    }
}
