// Log export service - core business logic
// Accessible from: CLI, in-app CLI, and UI Tauri commands

use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogExportRequest {
    pub output_path: Option<PathBuf>,
    pub format: LogFormat,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Text,
    Json,
}

impl From<&str> for LogFormat {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => LogFormat::Json,
            _ => LogFormat::Text,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogExportResult {
    pub success: bool,
    pub output_path: String,
    pub entries_count: usize,
    pub error_message: Option<String>,
}

/// Core log export operation
/// Called by: CLI handler, Tauri command, programmatic access
pub fn export_logs(request: LogExportRequest) -> LogExportResult {
    // Determine output path
    let output_path = request.output_path.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let ext = match request.format {
            LogFormat::Json => "json",
            LogFormat::Text => "txt",
        };
        PathBuf::from(format!("agentmux-logs-{}.{}", timestamp, ext))
    });

    // Collect log data
    let log_entries = collect_log_entries();

    // Write to file
    match write_log_file(&output_path, &request.format, &log_entries) {
        Ok(_) => LogExportResult {
            success: true,
            output_path: output_path.display().to_string(),
            entries_count: log_entries.len(),
            error_message: None,
        },
        Err(e) => LogExportResult {
            success: false,
            output_path: output_path.display().to_string(),
            entries_count: 0,
            error_message: Some(format!("Failed to write log file: {}", e)),
        },
    }
}

/// Collect log entries from various sources
fn collect_log_entries() -> Vec<serde_json::Value> {
    let mut entries = Vec::new();

    // 1. Runtime information
    entries.push(json!({
        "source": "runtime_info",
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "exe_path": std::env::current_exe().ok().map(|p| p.display().to_string()),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }));

    // 2. Collect from common log directories
    let potential_log_dirs = vec![
        // Windows: %LOCALAPPDATA%/agentmux-desktop/logs
        dirs::data_local_dir().map(|d| d.join("agentmux-desktop").join("logs")),
        // macOS/Linux: ~/.local/share/agentmux-desktop/logs
        dirs::data_dir().map(|d| d.join("agentmux-desktop").join("logs")),
        // Current directory
        Some(PathBuf::from("logs")),
    ];

    for log_dir in potential_log_dirs.into_iter().flatten() {
        if log_dir.exists() {
            if let Ok(dir_entries) = fs::read_dir(&log_dir) {
                for entry in dir_entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Ok(content) = fs::read_to_string(&path) {
                            entries.push(json!({
                                "source": "log_file",
                                "directory": log_dir.display().to_string(),
                                "file": entry.file_name().to_string_lossy().to_string(),
                                "content": content,
                                "timestamp": chrono::Utc::now().to_rfc3339(),
                            }));
                        }
                    }
                }
            }
        }
    }

    entries
}

/// Write log entries to file in specified format
fn write_log_file(
    path: &PathBuf,
    format: &LogFormat,
    entries: &[serde_json::Value],
) -> Result<(), std::io::Error> {
    match format {
        LogFormat::Json => {
            let output = json!({
                "export_timestamp": chrono::Utc::now().to_rfc3339(),
                "version": env!("CARGO_PKG_VERSION"),
                "log_entries": entries,
            });
            fs::write(path, serde_json::to_string_pretty(&output).unwrap())
        }
        LogFormat::Text => {
            let mut text = format!(
                "AgentMux Desktop Logs Export\n\
                 Version: {}\n\
                 Export Time: {}\n\
                 {}\n\n",
                env!("CARGO_PKG_VERSION"),
                chrono::Utc::now().to_rfc3339(),
                "=".repeat(60)
            );

            for entry in entries {
                let source = entry["source"].as_str().unwrap_or("unknown");
                text.push_str(&format!(
                    "Source: {}\n\
                     Timestamp: {}\n",
                    source,
                    entry["timestamp"].as_str().unwrap_or("unknown")
                ));

                if source == "log_file" {
                    if let Some(file) = entry["file"].as_str() {
                        text.push_str(&format!("File: {}\n", file));
                    }
                    if let Some(dir) = entry["directory"].as_str() {
                        text.push_str(&format!("Directory: {}\n", dir));
                    }
                }

                text.push_str(&format!("{}\n", "-".repeat(60)));

                if let Some(content) = entry["content"].as_str() {
                    text.push_str(content);
                    text.push('\n');
                } else if source == "runtime_info" {
                    text.push_str(&format!(
                        "Platform: {}\n\
                         Architecture: {}\n\
                         Executable: {}\n",
                        entry["platform"].as_str().unwrap_or("unknown"),
                        entry["arch"].as_str().unwrap_or("unknown"),
                        entry["exe_path"].as_str().unwrap_or("unknown")
                    ));
                }

                text.push('\n');
            }

            fs::write(path, text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_log_format_from_str() {
        assert_eq!(LogFormat::from("json"), LogFormat::Json);
        assert_eq!(LogFormat::from("JSON"), LogFormat::Json);
        assert_eq!(LogFormat::from("text"), LogFormat::Text);
        assert_eq!(LogFormat::from("txt"), LogFormat::Text);
        assert_eq!(LogFormat::from("unknown"), LogFormat::Text);
    }

    #[test]
    fn test_export_logs_text_format() {
        let temp_dir = std::env::temp_dir();
        let output_path = temp_dir.join("test-logs.txt");

        let request = LogExportRequest {
            output_path: Some(output_path.clone()),
            format: LogFormat::Text,
        };

        let result = export_logs(request);

        assert!(result.success, "Export should succeed");
        assert!(output_path.exists(), "Output file should exist");
        assert!(result.entries_count > 0, "Should have at least runtime info");

        // Cleanup
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_export_logs_json_format() {
        let temp_dir = std::env::temp_dir();
        let output_path = temp_dir.join("test-logs.json");

        let request = LogExportRequest {
            output_path: Some(output_path.clone()),
            format: LogFormat::Json,
        };

        let result = export_logs(request);

        assert!(result.success, "Export should succeed");
        assert!(output_path.exists(), "Output file should exist");

        // Verify JSON is valid
        let content = fs::read_to_string(&output_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["log_entries"].is_array());
        assert!(parsed["export_timestamp"].is_string());

        // Cleanup
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn test_export_logs_default_filename() {
        let request = LogExportRequest {
            output_path: None,
            format: LogFormat::Text,
        };

        let result = export_logs(request);

        assert!(result.success);
        assert!(result.output_path.starts_with("agentmux-logs-"));
        assert!(result.output_path.ends_with(".txt"));

        // Cleanup
        let _ = fs::remove_file(&result.output_path);
    }

    #[test]
    fn test_collect_log_entries() {
        let entries = collect_log_entries();

        // Should always have at least runtime info
        assert!(!entries.is_empty());

        // First entry should be runtime info
        assert_eq!(entries[0]["source"], "runtime_info");
        assert!(entries[0]["platform"].is_string());
        assert!(entries[0]["arch"].is_string());
    }
}
