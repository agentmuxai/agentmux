// CLI output formatting

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliResponse {
    pub success: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: String,
}

impl CliResponse {
    pub fn success(output: String, data: Option<Value>) -> Self {
        Self {
            success: true,
            output,
            data,
            error: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn error(error: String) -> Self {
        Self {
            success: false,
            output: format!("✗ {}", error),
            data: None,
            error: Some(error),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn format(&self, format: &OutputFormat) -> String {
        match format {
            OutputFormat::Text => self.output.clone(),
            OutputFormat::Json => serde_json::to_string_pretty(self).unwrap_or_else(|_| {
                format!(r#"{{"error":"Failed to serialize response"}}"#)
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
}

impl From<bool> for OutputFormat {
    fn from(json: bool) -> Self {
        if json {
            OutputFormat::Json
        } else {
            OutputFormat::Text
        }
    }
}
