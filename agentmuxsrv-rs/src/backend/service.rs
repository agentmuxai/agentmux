// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Service dispatcher: routes web/RPC calls to backend services.
//! Port of Go's pkg/service/service.go and all sub-services.
//!
//! Replaces Go's reflection-based dispatch with a match-based router.
//! Each service method is a typed function; argument conversion from
//! `serde_json::Value` is handled at the boundary.


use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::waveobj;

// ---- Wire types ----

/// Incoming service call from the frontend.
/// Matches Go's `service.WebCallType`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebCallType {
    pub service: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uicontext: Option<UIContext>,
    #[serde(default)]
    pub args: Vec<serde_json::Value>,
}

/// UI context passed with service calls.
/// Matches Go's `waveobj.UIContext`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIContext {
    #[serde(default, rename = "activetabid")]
    pub active_tab_id: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Service call response.
/// Matches Go's `service.WebReturnType`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebReturnType {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<Vec<waveobj::WaveObjUpdate>>,
}

impl WebReturnType {
    /// Create a success response with data.
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            success: true,
            error: None,
            data: Some(data),
            updates: None,
        }
    }

    /// Create a success response with no data.
    pub fn success_empty() -> Self {
        Self {
            success: true,
            error: None,
            data: None,
            updates: None,
        }
    }

    /// Create a success response with updates.
    pub fn success_with_updates(updates: Vec<waveobj::WaveObjUpdate>) -> Self {
        Self {
            success: true,
            error: None,
            data: None,
            updates: if updates.is_empty() {
                None
            } else {
                Some(updates)
            },
        }
    }

    /// Create a success response with both data and updates.
    pub fn success_data_updates(
        data: serde_json::Value,
        updates: Vec<waveobj::WaveObjUpdate>,
    ) -> Self {
        Self {
            success: true,
            error: None,
            data: Some(data),
            updates: if updates.is_empty() {
                None
            } else {
                Some(updates)
            },
        }
    }

    /// Create an error response.
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(msg.into()),
            data: None,
            updates: None,
        }
    }
}

// ---- WaveObjUpdate (matches Go's waveobj.WaveObjUpdate) ----
// This is re-exported from waveobj where it's defined.

// ---- Method metadata (for documentation and code generation) ----

/// Metadata about a service method. Matches Go's `tsgenmeta.MethodMeta`.
/// Used for TypeScript code generation and documentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arg_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_desc: Option<String>,
}

/// Registry of service method metadata (built at startup).
pub fn get_method_meta(service: &str, method: &str) -> Option<MethodMeta> {
    match (service, method) {
        // BlockService
        ("block", "SendCommand") => Some(MethodMeta {
            desc: Some("send command to block".into()),
            arg_names: vec!["blockid".into(), "cmd".into()],
            return_desc: None,
        }),
        ("block", "GetControllerStatus") => Some(MethodMeta {
            desc: Some("get block controller status".into()),
            arg_names: vec!["blockid".into()],
            return_desc: None,
        }),
        ("block", "SaveTerminalState") => Some(MethodMeta {
            desc: Some("save the terminal state to a blockfile".into()),
            arg_names: vec![
                "ctx".into(),
                "blockId".into(),
                "state".into(),
                "stateType".into(),
                "ptyOffset".into(),
                "termSize".into(),
            ],
            return_desc: None,
        }),

        // ObjectService
        ("object", "GetObject") => Some(MethodMeta {
            desc: Some("get wave object by oref".into()),
            arg_names: vec!["oref".into()],
            return_desc: None,
        }),
        ("object", "GetObjects") => Some(MethodMeta {
            desc: Some("get multiple wave objects".into()),
            arg_names: vec!["orefs".into()],
            return_desc: Some("objects".into()),
        }),
        ("object", "UpdateTabName") => Some(MethodMeta {
            desc: Some("update tab name".into()),
            arg_names: vec!["uiContext".into(), "tabId".into(), "name".into()],
            return_desc: None,
        }),
        ("object", "CreateBlock") => Some(MethodMeta {
            desc: Some("create a new block".into()),
            arg_names: vec!["uiContext".into(), "blockDef".into(), "rtOpts".into()],
            return_desc: Some("blockId".into()),
        }),
        ("object", "DeleteBlock") => Some(MethodMeta {
            desc: Some("delete a block".into()),
            arg_names: vec!["uiContext".into(), "blockId".into()],
            return_desc: None,
        }),
        ("object", "UpdateObjectMeta") => Some(MethodMeta {
            desc: Some("update object meta".into()),
            arg_names: vec!["uiContext".into(), "oref".into(), "meta".into()],
            return_desc: None,
        }),
        ("object", "UpdateObject") => Some(MethodMeta {
            desc: Some("update a wave object".into()),
            arg_names: vec!["uiContext".into(), "waveObj".into(), "returnUpdates".into()],
            return_desc: None,
        }),

        // ClientService
        ("client", "GetClientData") => Some(MethodMeta {
            desc: Some("get client data".into()),
            arg_names: vec![],
            return_desc: None,
        }),
        ("client", "GetTab") => Some(MethodMeta {
            desc: Some("get tab by ID".into()),
            arg_names: vec!["tabId".into()],
            return_desc: None,
        }),
        ("client", "GetAllConnStatus") => Some(MethodMeta {
            desc: Some("get all connection statuses".into()),
            arg_names: vec![],
            return_desc: None,
        }),
        ("client", "FocusWindow") => Some(MethodMeta {
            desc: Some("focus a window".into()),
            arg_names: vec!["windowId".into()],
            return_desc: None,
        }),
        ("client", "AgreeTos") => Some(MethodMeta {
            desc: Some("agree to terms of service".into()),
            arg_names: vec![],
            return_desc: None,
        }),
        ("client", "TelemetryUpdate") => Some(MethodMeta {
            desc: Some("update telemetry setting".into()),
            arg_names: vec!["telemetryEnabled".into()],
            return_desc: None,
        }),

        // WindowService
        ("window", "GetWindow") => Some(MethodMeta {
            desc: Some("get window by ID".into()),
            arg_names: vec!["windowId".into()],
            return_desc: None,
        }),
        ("window", "CreateWindow") => Some(MethodMeta {
            desc: Some("create a new window".into()),
            arg_names: vec!["ctx".into(), "winSize".into(), "workspaceId".into()],
            return_desc: None,
        }),
        ("window", "SetWindowPosAndSize") => Some(MethodMeta {
            desc: Some("set window position and size".into()),
            arg_names: vec!["ctx".into(), "windowId".into(), "pos".into(), "size".into()],
            return_desc: None,
        }),
        ("window", "MoveBlockToNewWindow") => Some(MethodMeta {
            desc: Some("move block to new window".into()),
            arg_names: vec!["ctx".into(), "currentTabId".into(), "blockId".into()],
            return_desc: None,
        }),
        ("window", "SwitchWorkspace") => Some(MethodMeta {
            desc: Some("switch workspace".into()),
            arg_names: vec!["ctx".into(), "windowId".into(), "workspaceId".into()],
            return_desc: None,
        }),
        ("window", "CloseWindow") => Some(MethodMeta {
            desc: Some("close a window".into()),
            arg_names: vec!["ctx".into(), "windowId".into(), "fromElectron".into()],
            return_desc: None,
        }),

        // WorkspaceService
        ("workspace", "CreateWorkspace") => Some(MethodMeta {
            desc: Some("create a new workspace".into()),
            arg_names: vec![
                "ctx".into(),
                "name".into(),
                "icon".into(),
                "color".into(),
                "applyDefaults".into(),
            ],
            return_desc: Some("workspaceId".into()),
        }),
        ("workspace", "UpdateWorkspace") => Some(MethodMeta {
            desc: Some("update workspace properties".into()),
            arg_names: vec![
                "ctx".into(),
                "workspaceId".into(),
                "name".into(),
                "icon".into(),
                "color".into(),
                "applyDefaults".into(),
            ],
            return_desc: None,
        }),
        ("workspace", "GetWorkspace") => Some(MethodMeta {
            desc: Some("get workspace by ID".into()),
            arg_names: vec!["workspaceId".into()],
            return_desc: Some("workspace".into()),
        }),
        ("workspace", "DeleteWorkspace") => Some(MethodMeta {
            desc: Some("delete a workspace".into()),
            arg_names: vec!["workspaceId".into()],
            return_desc: None,
        }),
        ("workspace", "ListWorkspaces") => Some(MethodMeta {
            desc: Some("list all workspaces".into()),
            arg_names: vec![],
            return_desc: None,
        }),
        ("workspace", "CreateTab") => Some(MethodMeta {
            desc: Some("create a new tab".into()),
            arg_names: vec![
                "workspaceId".into(),
                "tabName".into(),
                "activateTab".into(),
                "pinned".into(),
            ],
            return_desc: Some("tabId".into()),
        }),
        ("workspace", "GetColors") => Some(MethodMeta {
            desc: Some("get workspace colors".into()),
            arg_names: vec![],
            return_desc: Some("colors".into()),
        }),
        ("workspace", "GetIcons") => Some(MethodMeta {
            desc: Some("get workspace icons".into()),
            arg_names: vec![],
            return_desc: Some("icons".into()),
        }),
        ("workspace", "ChangeTabPinning") => Some(MethodMeta {
            desc: Some("change tab pinning state".into()),
            arg_names: vec![
                "ctx".into(),
                "workspaceId".into(),
                "tabId".into(),
                "pinned".into(),
            ],
            return_desc: None,
        }),
        ("workspace", "UpdateTabIds") => Some(MethodMeta {
            desc: Some("update tab ordering".into()),
            arg_names: vec![
                "uiContext".into(),
                "workspaceId".into(),
                "tabIds".into(),
                "pinnedTabIds".into(),
            ],
            return_desc: None,
        }),
        ("workspace", "SetActiveTab") => Some(MethodMeta {
            desc: Some("set active tab".into()),
            arg_names: vec!["workspaceId".into(), "tabId".into()],
            return_desc: None,
        }),
        ("workspace", "CloseTab") => Some(MethodMeta {
            desc: Some("close a tab".into()),
            arg_names: vec![
                "ctx".into(),
                "workspaceId".into(),
                "tabId".into(),
                "fromElectron".into(),
            ],
            return_desc: Some("CloseTabRtn".into()),
        }),

        // UserInputService
        ("userinput", "SendUserInputResponse") => Some(MethodMeta {
            desc: Some("send user input response".into()),
            arg_names: vec!["response".into()],
            return_desc: None,
        }),

        _ => None,
    }
}

// ---- Service-specific types ----

/// Return type from workspace CloseTab.
/// Matches Go's `workspaceservice.CloseTabRtnType`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseTabRtnType {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closewindow: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub newactivetabid: String,
}

/// List of available services. Matches Go's `ServiceMap` keys.
pub const SERVICES: &[&str] = &[
    "block",
    "object",
    "client",
    "window",
    "workspace",
    "userinput",
];

/// Check if a service name is valid.
pub fn is_valid_service(name: &str) -> bool {
    SERVICES.contains(&name)
}

/// Extract a typed argument from the args array.
pub fn get_arg<T: serde::de::DeserializeOwned>(
    args: &[serde_json::Value],
    idx: usize,
) -> Result<T, String> {
    let val = args
        .get(idx)
        .ok_or_else(|| format!("missing argument at index {}", idx))?;
    serde_json::from_value(val.clone()).map_err(|e| format!("invalid argument at index {}: {}", idx, e))
}

/// Extract an optional typed argument from the args array.
/// Returns Ok(None) if the index is out of bounds or the value is null.
pub fn get_optional_arg<T: serde::de::DeserializeOwned>(
    args: &[serde_json::Value],
    idx: usize,
) -> Result<Option<T>, String> {
    match args.get(idx) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(val) => serde_json::from_value(val.clone())
            .map(Some)
            .map_err(|e| format!("invalid argument at index {}: {}", idx, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_call_type_deserialize() {
        let json = r#"{
            "service": "object",
            "method": "GetObject",
            "args": ["block:abc123"]
        }"#;
        let call: WebCallType = serde_json::from_str(json).unwrap();
        assert_eq!(call.service, "object");
        assert_eq!(call.method, "GetObject");
        assert_eq!(call.args.len(), 1);
        assert!(call.uicontext.is_none());
    }

    #[test]
    fn test_web_call_type_with_uicontext() {
        let json = r#"{
            "service": "object",
            "method": "CreateBlock",
            "uicontext": {"activetabid": "tab-123"},
            "args": [{"view": "term"}, {}]
        }"#;
        let call: WebCallType = serde_json::from_str(json).unwrap();
        assert_eq!(call.service, "object");
        assert_eq!(call.method, "CreateBlock");
        assert_eq!(call.uicontext.as_ref().unwrap().active_tab_id, "tab-123");
        assert_eq!(call.args.len(), 2);
    }

    #[test]
    fn test_web_return_type_success() {
        let rtn = WebReturnType::success(serde_json::json!("hello"));
        assert!(rtn.success);
        assert!(rtn.error.is_none());
        assert_eq!(rtn.data, Some(serde_json::json!("hello")));
    }

    #[test]
    fn test_web_return_type_error() {
        let rtn = WebReturnType::error("something went wrong");
        assert!(!rtn.success);
        assert_eq!(rtn.error.as_deref(), Some("something went wrong"));
        assert!(rtn.data.is_none());
    }

    #[test]
    fn test_web_return_type_success_empty() {
        let rtn = WebReturnType::success_empty();
        assert!(rtn.success);
        assert!(rtn.data.is_none());
        assert!(rtn.updates.is_none());
    }

    #[test]
    fn test_web_return_type_with_updates() {
        let updates = vec![waveobj::WaveObjUpdate {
            updatetype: "update".into(),
            otype: "tab".into(),
            oid: "123".into(),
            obj: None,
        }];
        let rtn = WebReturnType::success_with_updates(updates);
        assert!(rtn.success);
        assert_eq!(rtn.updates.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_web_return_type_empty_updates_omitted() {
        let rtn = WebReturnType::success_with_updates(vec![]);
        let json = serde_json::to_string(&rtn).unwrap();
        assert!(!json.contains("updates"));
    }

    #[test]
    fn test_web_return_type_serde_roundtrip() {
        let rtn = WebReturnType::success_data_updates(
            serde_json::json!({"id": "block-1"}),
            vec![waveobj::WaveObjUpdate {
                updatetype: "update".into(),
                otype: "block".into(),
                oid: "abc".into(),
                obj: Some(serde_json::json!({"otype": "block"})),
            }],
        );
        let json = serde_json::to_string(&rtn).unwrap();
        let parsed: WebReturnType = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert!(parsed.data.is_some());
        assert_eq!(parsed.updates.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_close_tab_rtn_type() {
        let rtn = CloseTabRtnType {
            closewindow: true,
            newactivetabid: String::new(),
        };
        let json = serde_json::to_string(&rtn).unwrap();
        assert!(json.contains("closewindow"));
        assert!(!json.contains("newactivetabid")); // empty string skipped
    }

    #[test]
    fn test_method_meta_exists() {
        assert!(get_method_meta("object", "GetObject").is_some());
        assert!(get_method_meta("workspace", "CreateTab").is_some());
        assert!(get_method_meta("block", "SaveTerminalState").is_some());
        assert!(get_method_meta("nonexistent", "Foo").is_none());
    }

    #[test]
    fn test_is_valid_service() {
        assert!(is_valid_service("block"));
        assert!(is_valid_service("object"));
        assert!(is_valid_service("workspace"));
        assert!(!is_valid_service("invalid"));
        assert!(!is_valid_service(""));
    }

    #[test]
    fn test_get_arg() {
        let args = vec![
            serde_json::json!("hello"),
            serde_json::json!(42),
            serde_json::json!(true),
        ];
        assert_eq!(get_arg::<String>(&args, 0).unwrap(), "hello");
        assert_eq!(get_arg::<i64>(&args, 1).unwrap(), 42);
        assert_eq!(get_arg::<bool>(&args, 2).unwrap(), true);
        assert!(get_arg::<String>(&args, 5).is_err());
    }

    #[test]
    fn test_get_optional_arg() {
        let args = vec![
            serde_json::json!("hello"),
            serde_json::Value::Null,
        ];
        assert_eq!(
            get_optional_arg::<String>(&args, 0).unwrap(),
            Some("hello".to_string())
        );
        assert_eq!(get_optional_arg::<String>(&args, 1).unwrap(), None);
        assert_eq!(get_optional_arg::<String>(&args, 5).unwrap(), None);
    }

    #[test]
    fn test_ui_context_deserialize() {
        let json = r#"{"activetabid": "tab-1", "extra_field": true}"#;
        let ctx: UIContext = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.active_tab_id, "tab-1");
        assert_eq!(ctx.extra.get("extra_field"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn test_web_call_no_args() {
        let json = r#"{"service": "client", "method": "GetClientData"}"#;
        let call: WebCallType = serde_json::from_str(json).unwrap();
        assert_eq!(call.service, "client");
        assert!(call.args.is_empty());
    }
}
