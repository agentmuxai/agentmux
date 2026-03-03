// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Crash handling for AgentMux Tauri.
// Replaces emain/crash-handlers.ts and emain/crash-reporter.ts

use std::panic;
use std::path::PathBuf;

/// Initialize the panic hook to capture and log crash information.
///
/// On panic, this will:
/// 1. Log the panic message via tracing
/// 2. Write a crash report to the app data directory
/// 3. Attempt to show a native error dialog (if available)
pub fn init_crash_handler(log_dir: PathBuf) {
    let crash_log_dir = log_dir.clone();

    panic::set_hook(Box::new(move |panic_info| {
        let msg = format!("PANIC: {}", panic_info);

        // Log via tracing (will go to both file and stderr)
        tracing::error!("{}", msg);

        // Write crash report to file
        let crash_file = crash_log_dir.join(format!(
            "crash-{}.log",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));

        let crash_report = format!(
            "AgentMux Crash Report\n\
             ===================\n\
             Time: {}\n\
             Panic: {}\n\
             \n\
             Location: {}\n\
             \n\
             Please report this at: https://github.com/agentmuxhq/agentmux/issues\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            panic_info,
            panic_info.location().map_or("unknown".to_string(), |loc| {
                format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
            })
        );

        if let Err(e) = std::fs::write(&crash_file, crash_report) {
            eprintln!("Failed to write crash report: {}", e);
        } else {
            eprintln!("Crash report written to: {}", crash_file.display());
        }
    }));
}
