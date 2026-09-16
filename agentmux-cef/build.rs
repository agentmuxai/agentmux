// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Windows application manifest. The `supportedOS` GUIDs (Vista, 7, 8, 8.1,
// 10/11) make Windows report the true OS version to the process instead of
// capping at 6.2 — required for correct Chromium GPU initialization. Mirrors
// the manifest Electron embeds. `asInvoker` preserves winres's default UAC
// behavior (no elevation prompt).
#[cfg(target_os = "windows")]
const WINDOWS_APP_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{e2011457-1546-43c5-a5fe-008deee3d3f0}"/>
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

fn main() {
    // No blanket `cargo:rerun-if-changed` directive by design: emitting even
    // one disables Cargo's default "rerun build.rs when any package file
    // changes" and replaces it with exactly the paths listed. That default is
    // exactly what keeps AGENTMUX_GIT_HASH / AGENTMUX_BUILD_TIME honest —
    // build.rs re-runs whenever agentmux-cef is rebuilt, so the embedded
    // metadata always describes the binary just produced. A commit that
    // touches no agentmux-cef file yields no new binary; the prior binary
    // (carrying its own correct metadata) is what continues to run.
    //
    // AGENTMUX_CEF_VERSION below reads a file OUTSIDE this package
    // (../Cargo.lock), which the implicit default does not cover — it only
    // watches this crate's own directory. So we replicate that default
    // explicitly (`.`) and add the lockfile alongside it: a `cef` version
    // bump that touches only Cargo.lock, with no other agentmux-cef file
    // changed, would otherwise rebuild against the new crate while reusing
    // the stale cached AGENTMUX_CEF_VERSION (codex P2, PR #3266).
    println!("cargo:rerun-if-changed=.");
    println!("cargo:rerun-if-changed=../Cargo.lock");

    // Emit the target triple so we can locate sidecar binaries at runtime.
    println!(
        "cargo:rustc-env=AGENTMUX_TARGET_TRIPLE={}",
        std::env::var("TARGET").unwrap()
    );

    // Embed the git short hash + build timestamp for the Instance panel's
    // Build / Time rows. Falls back to "unknown" / 0 when git is unavailable
    // (e.g. building from a source tarball) — never fails the build.
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=AGENTMUX_GIT_HASH={git_hash}");

    let build_time_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    println!("cargo:rustc-env=AGENTMUX_BUILD_TIME={build_time_ms}");

    // Embed the linked `cef` crate's version (e.g. "152.1.0+152.0.6" — the
    // part after `+` is the actual CEF binary release) for the Instance
    // panel's CEF row. Read directly from the workspace Cargo.lock rather
    // than shelling out to `cargo metadata` (what
    // scripts/verify-cef-version.sh does) — this runs on every agentmux-cef
    // rebuild, and a lockfile scan is near-instant where a metadata
    // subprocess is not. Falls back to "unknown" rather than failing the
    // build if the lockfile ever moves or the entry isn't found.
    let cef_version = std::fs::read_to_string("../Cargo.lock")
        .ok()
        .and_then(|lock| {
            let idx = lock.find("name = \"cef\"")?;
            let after = &lock[idx..];
            let line = after.lines().nth(1)?; // the `version = "..."` line right after `name = "cef"`
            line.trim().strip_prefix("version = \"")?.strip_suffix('"').map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=AGENTMUX_CEF_VERSION={cef_version}");

    // Windows: embed application icon + version info into PE VERSIONINFO resource.
    // FileDescription controls the "Name" column in Task Manager's Processes tab.
    // The actual exe filename controls WER crash dump names and Event Viewer entries.
    #[cfg(target_os = "windows")]
    {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", &format!("AgentMux v{}", version));
        res.set("ProductName", "AgentMux");
        res.set("CompanyName", "AgentMux");
        res.set("InternalName", "agentmux");
        let icon_path = std::path::Path::new("resources/win/agentmux.ico");
        if icon_path.exists() {
            res.set_icon(icon_path.to_str().unwrap());
        }
        // Embed an application manifest declaring supportedOS for Win7→Win10/11.
        // Without it the exe is "unmanifested": Windows lies about its version and
        // reports 6.2 (Windows 8) via GetVersionEx — which sends Chromium's GPU
        // init down OS-version-gated paths (DirectComposition / D3D feature
        // detection / driver workarounds) that CHECK-crash the GPU process at
        // "create shared context for virtualization" on modern Win11/multi-GPU/
        // virtual-display hardware. chrome://gpu showed `Windows NT 6.2.9200`
        // before this; Electron/VS Code ship the same manifest and report 10.0.
        // The GPU process runs as THIS exe (agentmux-cef.exe --type=gpu-process),
        // so its manifest is what determines the reported OS version.
        res.set_manifest(WINDOWS_APP_MANIFEST);
        res.compile().expect("winres compile failed");
    }
}
