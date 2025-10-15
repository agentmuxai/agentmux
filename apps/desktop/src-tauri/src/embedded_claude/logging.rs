// Logging utility for Embedded Claude with taxonomy and colored output
//
// This module provides structured logging that emits to both:
// 1. Tauri event system (for UI DebugConsole)
// 2. Terminal output (for development)
//
// Taxonomy:
// - PROCESS: Process lifecycle (spawn, exit, signals)
// - STDIN: Input handling to Claude process
// - STDOUT: Output streaming from Claude process
// - STDERR: Error output from Claude process
// - WEBSOCKET: WebSocket server and connections
// - MESSAGE: File-based message watching and routing
// - STATE: State management and coordination
// - ERROR: Critical errors and failures

use tauri::{AppHandle, Emitter};
use serde::Serialize;

/// Log level with associated color (using ANSI color codes for terminal)
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Debug information (gray)
    Debug,
    /// Informational messages (blue)
    Info,
    /// Success messages (green)
    Success,
    /// Warning messages (yellow)
    Warning,
    /// Error messages (red)
    Error,
}

impl LogLevel {
    /// Get ANSI color code for terminal output
    fn color_code(&self) -> &'static str {
        match self {
            LogLevel::Debug => "\x1b[90m",     // Gray
            LogLevel::Info => "\x1b[34m",      // Blue
            LogLevel::Success => "\x1b[32m",   // Green
            LogLevel::Warning => "\x1b[33m",   // Yellow
            LogLevel::Error => "\x1b[31m",     // Red
        }
    }

    /// Get color reset code
    fn reset_code() -> &'static str {
        "\x1b[0m"
    }

    /// Get emoji prefix for UI display
    fn emoji(&self) -> &'static str {
        match self {
            LogLevel::Debug => "🔍",
            LogLevel::Info => "ℹ️",
            LogLevel::Success => "✅",
            LogLevel::Warning => "⚠️",
            LogLevel::Error => "❌",
        }
    }

    /// Get CSS color for UI display
    pub fn css_color(&self) -> &'static str {
        match self {
            LogLevel::Debug => "#888888",
            LogLevel::Info => "#4A90E2",
            LogLevel::Success => "#7ED321",
            LogLevel::Warning => "#F5A623",
            LogLevel::Error => "#D0021B",
        }
    }
}

/// Log category/taxonomy
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogCategory {
    Process,
    Stdin,
    Stdout,
    Stderr,
    WebSocket,
    Message,
    State,
    Error,
}

impl LogCategory {
    /// Get icon for category
    fn icon(&self) -> &'static str {
        match self {
            LogCategory::Process => "⚙️",
            LogCategory::Stdin => "📥",
            LogCategory::Stdout => "📤",
            LogCategory::Stderr => "🚨",
            LogCategory::WebSocket => "🔌",
            LogCategory::Message => "💬",
            LogCategory::State => "📊",
            LogCategory::Error => "🔥",
        }
    }
}

/// Structured log entry
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub level: LogLevel,
    pub category: LogCategory,
    pub instance: Option<String>,
    pub message: String,
    pub timestamp: String,
}

impl LogEntry {
    /// Format for terminal output with colors
    fn format_terminal(&self) -> String {
        let timestamp = chrono::Local::now().format("%H:%M:%S%.3f");
        let level_str = format!("{:?}", self.level).to_uppercase();
        let category_str = format!("{:?}", self.category).to_uppercase();

        let instance_part = if let Some(ref inst) = self.instance {
            format!("[{}]", inst)
        } else {
            String::new()
        };

        format!(
            "{}[{}] {}{} {} {}{} {}",
            LogLevel::Debug.color_code(),
            timestamp,
            self.level.emoji(),
            level_str,
            self.category.icon(),
            category_str,
            instance_part,
            self.message,
        ) + LogLevel::reset_code()
    }

    /// Format for UI display
    fn format_ui(&self) -> String {
        let instance_part = if let Some(ref inst) = self.instance {
            format!("[{}] ", inst)
        } else {
            String::new()
        };

        format!(
            "{} {} {}{}: {}",
            self.level.emoji(),
            self.category.icon(),
            instance_part,
            format!("{:?}", self.category).to_uppercase(),
            self.message
        )
    }
}

/// Main logging function
pub fn log(
    app_handle: &AppHandle,
    level: LogLevel,
    category: LogCategory,
    instance: Option<&str>,
    message: impl Into<String>,
) {
    let entry = LogEntry {
        level,
        category,
        instance: instance.map(|s| s.to_string()),
        message: message.into(),
        timestamp: chrono::Local::now().to_rfc3339(),
    };

    // Print to terminal with colors
    println!("{}", entry.format_terminal());

    // Emit to UI DebugConsole
    let ui_message = entry.format_ui();
    let _ = app_handle.emit("debug_log", ui_message);
}

/// Convenience macros for each log level

pub fn debug(
    app_handle: &AppHandle,
    category: LogCategory,
    instance: Option<&str>,
    message: impl Into<String>,
) {
    log(app_handle, LogLevel::Debug, category, instance, message);
}

pub fn info(
    app_handle: &AppHandle,
    category: LogCategory,
    instance: Option<&str>,
    message: impl Into<String>,
) {
    log(app_handle, LogLevel::Info, category, instance, message);
}

pub fn success(
    app_handle: &AppHandle,
    category: LogCategory,
    instance: Option<&str>,
    message: impl Into<String>,
) {
    log(app_handle, LogLevel::Success, category, instance, message);
}

pub fn warning(
    app_handle: &AppHandle,
    category: LogCategory,
    instance: Option<&str>,
    message: impl Into<String>,
) {
    log(app_handle, LogLevel::Warning, category, instance, message);
}

pub fn error(
    app_handle: &AppHandle,
    category: LogCategory,
    instance: Option<&str>,
    message: impl Into<String>,
) {
    log(app_handle, LogLevel::Error, category, instance, message);
}

// Specialized logging functions for common patterns

/// Log process spawn
pub fn log_process_spawn(app_handle: &AppHandle, instance: &str, command: &str) {
    info(
        app_handle,
        LogCategory::Process,
        Some(instance),
        format!("Spawning process: {}", command),
    );
}

/// Log process exit
pub fn log_process_exit(app_handle: &AppHandle, instance: &str, exit_code: Option<i32>) {
    let message = match exit_code {
        Some(code) => format!("Process exited with code: {}", code),
        None => "Process terminated by signal".to_string(),
    };

    if exit_code == Some(0) {
        success(app_handle, LogCategory::Process, Some(instance), message);
    } else {
        error(app_handle, LogCategory::Process, Some(instance), message);
    }
}

/// Log stdin write
pub fn log_stdin_write(app_handle: &AppHandle, instance: &str, count: usize, bytes: usize) {
    info(
        app_handle,
        LogCategory::Stdin,
        Some(instance),
        format!("→ Sending input #{} ({} bytes)", count, bytes),
    );
}

/// Log stdin write error
pub fn log_stdin_error(app_handle: &AppHandle, instance: &str, count: usize, err: &str) {
    error(
        app_handle,
        LogCategory::Stdin,
        Some(instance),
        format!("✗ Failed to write input #{}: {}", count, err),
    );
}

/// Log stdout line
pub fn log_stdout_line(app_handle: &AppHandle, instance: &str, line: &str) {
    info(
        app_handle,
        LogCategory::Stdout,
        Some(instance),
        format!("← {}", line),
    );
}

/// Log stderr line
pub fn log_stderr_line(app_handle: &AppHandle, instance: &str, line: &str) {
    warning(
        app_handle,
        LogCategory::Stderr,
        Some(instance),
        format!("⚠ {}", line),
    );
}

/// Log WebSocket connection
pub fn log_ws_connection(app_handle: &AppHandle, instance: &str, addr: &str, connected: bool) {
    if connected {
        success(
            app_handle,
            LogCategory::WebSocket,
            Some(instance),
            format!("Client connected: {}", addr),
        );
    } else {
        info(
            app_handle,
            LogCategory::WebSocket,
            Some(instance),
            format!("Client disconnected: {}", addr),
        );
    }
}

/// Log message file event
pub fn log_message_event(app_handle: &AppHandle, instance: &str, event: &str, path: &str) {
    info(
        app_handle,
        LogCategory::Message,
        Some(instance),
        format!("📂 {} → {}", event, path),
    );
}

/// Log message routing decision
pub fn log_message_routing(app_handle: &AppHandle, instance: &str, matched: bool, reason: &str) {
    if matched {
        success(
            app_handle,
            LogCategory::Message,
            Some(instance),
            format!("✓ Message matched: {}", reason),
        );
    } else {
        debug(
            app_handle,
            LogCategory::Message,
            Some(instance),
            format!("✗ Message skipped: {}", reason),
        );
    }
}

/// Log state change
pub fn log_state_change(app_handle: &AppHandle, instance: Option<&str>, change: &str) {
    info(app_handle, LogCategory::State, instance, format!("State: {}", change));
}
