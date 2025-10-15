// Logs export command

use agentmux_desktop::services::logs::{export_logs as export_logs_service, LogExportRequest, LogFormat};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

#[tauri::command]
pub async fn export_logs(
    output_path: Option<String>,
    format: String,
    app_handle: AppHandle,
) -> Result<String, String> {
    let request = LogExportRequest {
        output_path: output_path.map(PathBuf::from),
        format: LogFormat::from(format.as_str()),
    };

    let result = export_logs_service(request);

    if result.success {
        // Emit event for UI reactivity
        let _ = app_handle.emit("logs_exported", serde_json::json!({
            "output_path": result.output_path.clone(),
            "format": format,
            "entries_count": result.entries_count,
            "success": true
        }));

        Ok(serde_json::json!({
            "output_path": result.output_path,
            "entries_count": result.entries_count,
        })
        .to_string())
    } else {
        Err(result
            .error_message
            .unwrap_or_else(|| "Unknown error".to_string()))
    }
}
