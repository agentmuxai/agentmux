// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hide console window in release mode on Windows (sandbox-off path only).
// The DLL path (Phase 3) uses bootstrap.exe which is already /SUBSYSTEM:WINDOWS.
#![cfg_attr(
    all(not(debug_assertions), not(feature = "sandbox"), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    std::process::exit(agentmux_cef::run(std::ptr::null_mut()));
}
