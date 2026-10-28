// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The `Drive` tool's read actions (G2), named after Google's Drive MCP tools
//! (SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §5):
//! `search_files`, `get_file_metadata`, `read_file_content`. On `drive.file` an
//! account reaches only the files the person picked (Google's desktop picker,
//! §3.3) and the ones AgentMux made, so a search is over those and nothing else.
//!
//! As with Calendar, each action is a [`plan`] (the request, from the agent's
//! arguments) and a [`shape`] (Google's answer, trimmed and capped); neither
//! touches the network or a token. `read_file_content` takes two requests: the
//! file's metadata decides whether it is exported ([`content_plan`]).

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{clip, GoogleRequest, SCOPE_PREFIX, TEXT_CAP};

const API: &str = "https://www.googleapis.com/drive/v3";

/// The read actions, in the order the tool lists them. `pick_files` is not one: it asks
/// the person, and srv answers it itself.
pub const READ_ACTIONS: [&str; 3] = ["search_files", "get_file_metadata", "read_file_content"];

const DEFAULT_RESULTS: u32 = 25;
const MAX_RESULTS: u32 = 100;
/// The fields of a file Google is asked for.
const FILE_FIELDS: &str = "id,name,mimeType,modifiedTime,size,webViewLink,owners(emailAddress)";

/// The arguments the read actions take. Unknown fields are ignored.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct DriveArgs {
    /// `get_file_metadata`, `read_file_content`: a file id from `search_files`.
    #[serde(default)]
    pub file_id: Option<String>,
    /// `search_files`: words to look for in names and contents.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub page_token: Option<String>,
}

/// The scopes an action can run with: any one is enough. `drive.readonly` and `drive` are
/// never asked for here, but an account connected with them still works.
pub fn needs(_action: &str) -> &'static [&'static str] {
    &["drive.file", "drive.readonly", "drive"]
}

/// Whether `granted` (full scope URLs, or bare names) covers `action`.
pub fn granted_for(action: &str, granted: &[String]) -> bool {
    needs(action)
        .iter()
        .any(|n| granted.iter().any(|g| g.strip_prefix(SCOPE_PREFIX).unwrap_or(g) == *n))
}

fn file_id(args: &DriveArgs) -> Result<String, String> {
    let id = args.file_id.as_deref().map(str::trim).unwrap_or("");
    if id.is_empty() {
        return Err("this action needs file_id: a file id from search_files".to_string());
    }
    if id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(format!("\"{}\" isn't a Drive file id", clip(id, 40)));
    }
    Ok(id.to_string())
}

fn file_url(id: &str, tail: Option<&str>) -> Result<String, String> {
    let mut url = url::Url::parse(API).map_err(|e| e.to_string())?;
    {
        let mut seg = url.path_segments_mut().map_err(|_| "bad Drive API URL".to_string())?;
        seg.push("files").push(id);
        if let Some(t) = tail {
            seg.push(t);
        }
    }
    Ok(url.to_string())
}

/// A string for Drive's `q`: backslashes and single quotes escaped.
fn quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn get(url: String, query: Vec<(&'static str, String)>) -> GoogleRequest {
    GoogleRequest { method: reqwest::Method::GET, url, query, body: None, text: false }
}

/// The request to Google for a read action; for `read_file_content`, the file's metadata,
/// which [`content_plan`] turns into the request for its text.
pub fn plan(action: &str, args: &DriveArgs) -> Result<GoogleRequest, String> {
    match action {
        "search_files" => {
            let text = args.query.as_deref().map(str::trim).unwrap_or("");
            let q = if text.is_empty() {
                "trashed = false".to_string()
            } else {
                format!("fullText contains '{}' and trashed = false", quoted(&clip(text, 200)))
            };
            let mut query = vec![
                ("q", q),
                ("pageSize", args.max_results.unwrap_or(DEFAULT_RESULTS).clamp(1, MAX_RESULTS).to_string()),
                ("fields", format!("nextPageToken,files({FILE_FIELDS})")),
            ];
            if text.is_empty() {
                query.push(("orderBy", "modifiedTime desc".to_string()));
            }
            if let Some(t) = args.page_token.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                query.push(("pageToken", t.to_string()));
            }
            Ok(get(format!("{API}/files"), query))
        }
        "get_file_metadata" | "read_file_content" => {
            Ok(get(file_url(&file_id(args)?, None)?, vec![("fields", FILE_FIELDS.to_string())]))
        }
        other => Err(format!("Drive has no action \"{other}\"; one of {}, pick_files", READ_ACTIONS.join(", "))),
    }
}

/// How a file is read as text: Google's native documents are exported, plain text is
/// downloaded, anything else can't be.
pub fn content_plan(meta: &Value) -> Result<GoogleRequest, String> {
    let id = s(meta, "/id");
    let mime = s(meta, "/mimeType");
    let export = match mime {
        "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => Some("text/plain"),
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        _ => None,
    };
    let mut req = if let Some(to) = export {
        get(file_url(id, Some("export"))?, vec![("mimeType", to.to_string())])
    } else if is_text(mime) {
        get(file_url(id, None)?, vec![("alt", "media".to_string())])
    } else {
        return Err(format!(
            "{} is a {} file, which can't be read as text here. Only Google Docs, Sheets and Slides (exported) and plain-text files can",
            clip(s(meta, "/name"), 80),
            if mime.is_empty() { "unknown-type" } else { mime }
        ));
    };
    req.text = true;
    Ok(req)
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/x-yaml" | "application/yaml" | "application/javascript" | "application/x-sh"
        )
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
}

fn s<'a>(v: &'a Value, ptr: &str) -> &'a str {
    v.pointer(ptr).and_then(Value::as_str).unwrap_or("")
}

fn shape_file(f: &Value) -> Value {
    let mut out = Map::new();
    for (src, dst) in [
        ("/id", "id"),
        ("/name", "name"),
        ("/mimeType", "mime_type"),
        ("/modifiedTime", "modified"),
        ("/size", "size"),
        ("/webViewLink", "link"),
        ("/owners/0/emailAddress", "owner"),
    ] {
        let v = s(f, src);
        if !v.is_empty() {
            out.insert(dst.to_string(), json!(v));
        }
    }
    Value::Object(out)
}

/// Google's answer to `search_files` or `get_file_metadata`, trimmed for the agent.
pub fn shape(action: &str, body: &Value) -> Value {
    match action {
        "search_files" => {
            let files: Vec<Value> = body.get("files").and_then(Value::as_array).map(|a| a.iter().map(shape_file).collect()).unwrap_or_default();
            let mut out = Map::new();
            out.insert("files".into(), json!(files));
            let t = s(body, "/nextPageToken");
            if !t.is_empty() {
                out.insert("next_page_token".into(), json!(t));
            }
            Value::Object(out)
        }
        "get_file_metadata" => shape_file(body),
        _ => Value::Null,
    }
}

/// `read_file_content`'s answer: the file's details and its text, as far as it was read.
pub fn shape_content(meta: &Value, reply: &Value) -> Value {
    let mut out = Map::new();
    out.insert("file".into(), shape_file(meta));
    out.insert("content".into(), json!(s(reply, "/text")));
    if reply.get("truncated").and_then(Value::as_bool) == Some(true) {
        out.insert("truncated".into(), json!(format!("only the first {} bytes of the file are shown", TEXT_CAP)));
    }
    if s(meta, "/mimeType") == "application/vnd.google-apps.spreadsheet" {
        out.insert("note".into(), json!("a Sheet is exported as CSV: its first sheet only"));
    }
    Value::Object(out)
}

/// The ids a read touched, for the account's log (D7): file ids, never what was in them.
pub fn logged_ids(action: &str, args: &DriveArgs) -> String {
    match action {
        "get_file_metadata" | "read_file_content" => args.file_id.as_deref().map(str::trim).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;
