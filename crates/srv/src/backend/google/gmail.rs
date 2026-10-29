// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The `Gmail` tool's read actions (G3), named after Google's Gmail MCP tools
//! (SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §5):
//! `search_threads`, `get_thread`, `list_labels`.
//!
//! As with Calendar and Drive, each action is a [`plan`] (the request, from
//! the agent's arguments) and a [`shape`] (Google's answer, trimmed and
//! capped); neither touches the network or a token. Google's thread search
//! answers with ids and a snippet only, so `search_threads` takes one more
//! request per thread shown ([`summary_plan`]) for who wrote it, the subject
//! and the date. Mail is written by other people: every text that comes back
//! is cut to a length and the tool marks it untrusted (D6).

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{clip, GoogleRequest, SCOPE_PREFIX};

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// The read actions, in the order the tool lists them.
pub const READ_ACTIONS: [&str; 3] = ["search_threads", "get_thread", "list_labels"];

const DEFAULT_RESULTS: u32 = 10;
const MAX_RESULTS: u32 = 25;
/// Threads of a search that get a subject and sender (one request each).
pub const SUMMARIZED: usize = 10;
/// A message's text, a thread's messages and the text of a whole thread are cut to these.
const BODY_CHARS: usize = 20_000;
const MESSAGES: usize = 30;
const THREAD_CHARS: usize = 120_000;
const SNIPPET_CHARS: usize = 300;
const ADDRESS_CHARS: usize = 300;
const ATTACHMENTS: usize = 20;

/// The arguments the read actions take. Unknown fields are ignored.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct GmailArgs {
    /// `search_threads`: Gmail's own search syntax (`from:alice newer_than:7d`).
    #[serde(default)]
    pub query: Option<String>,
    /// `search_threads`: only threads with this label id (from `list_labels`).
    #[serde(default)]
    pub label_id: Option<String>,
    /// `get_thread`: a thread id from `search_threads`.
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
    #[serde(default)]
    pub page_token: Option<String>,
}

/// The scopes an action can run with: any one is enough. Only `gmail.readonly` is asked for
/// here, but an account connected with a wider Gmail scope still works.
pub fn needs(_action: &str) -> &'static [&'static str] {
    &["gmail.readonly", "gmail.modify", "gmail.metadata"]
}

/// Whether `granted` (full scope URLs, or bare names) covers `action`. `gmail.metadata` can't
/// search or read bodies, so it covers `list_labels` alone.
pub fn granted_for(action: &str, granted: &[String]) -> bool {
    let has = |n: &str| granted.iter().any(|g| g.strip_prefix(SCOPE_PREFIX).unwrap_or(g) == n);
    match action {
        "list_labels" => needs(action).iter().any(|n| has(n)),
        _ => has("gmail.readonly") || has("gmail.modify"),
    }
}

/// An id in a URL path: Gmail's are hex, but accept letters, digits, `-` and `_`.
fn id_arg(name: &str, v: Option<&str>) -> Result<String, String> {
    let id = v.map(str::trim).unwrap_or("");
    if id.is_empty() {
        return Err(format!("this action needs {name}: one from search_threads"));
    }
    if id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(format!("\"{}\" isn't a Gmail id", clip(id, 40)));
    }
    Ok(id.to_string())
}

fn url_of(tail: &[&str]) -> Result<String, String> {
    let mut url = url::Url::parse(API).map_err(|e| e.to_string())?;
    url.path_segments_mut().map_err(|_| "bad Gmail API URL".to_string())?.extend(tail);
    Ok(url.to_string())
}

fn get(url: String, query: Vec<(&'static str, String)>) -> GoogleRequest {
    GoogleRequest { method: reqwest::Method::GET, url, query, body: None, text: false }
}

/// The request to Google for a read action.
pub fn plan(action: &str, args: &GmailArgs) -> Result<GoogleRequest, String> {
    match action {
        "search_threads" => {
            let mut q = vec![("maxResults", args.max_results.unwrap_or(DEFAULT_RESULTS).clamp(1, MAX_RESULTS).to_string())];
            if let Some(text) = args.query.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                q.push(("q", clip(text, 500)));
            }
            if let Some(l) = args.label_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                q.push(("labelIds", clip(l, 100)));
            }
            if let Some(t) = args.page_token.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                q.push(("pageToken", t.to_string()));
            }
            Ok(get(url_of(&["threads"])?, q))
        }
        "get_thread" => Ok(get(url_of(&["threads", &id_arg("thread_id", args.thread_id.as_deref())?])?, vec![("format", "full".to_string())])),
        "list_labels" => Ok(get(url_of(&["labels"])?, Vec::new())),
        other => Err(format!("Gmail has no action \"{other}\"; one of {}", READ_ACTIONS.join(", "))),
    }
}

/// The request for a found thread's headers, for the line `search_threads` shows.
pub fn summary_plan(thread_id: &str) -> Result<GoogleRequest, String> {
    let id = id_arg("thread_id", Some(thread_id))?;
    let mut q = vec![("format", "metadata".to_string())];
    for h in ["From", "Subject", "Date"] {
        q.push(("metadataHeaders", h.to_string()));
    }
    Ok(get(url_of(&["threads", &id])?, q))
}

fn s<'a>(v: &'a Value, ptr: &str) -> &'a str {
    v.pointer(ptr).and_then(Value::as_str).unwrap_or("")
}

/// A header's value (names compare case-insensitively), or empty.
fn header<'a>(payload: &'a Value, name: &str) -> &'a str {
    payload
        .get("headers")
        .and_then(Value::as_array)
        .and_then(|h| h.iter().find(|h| s(h, "/name").eq_ignore_ascii_case(name)))
        .map(|h| s(h, "/value"))
        .unwrap_or("")
}

/// Header values hold what the sender typed: one line, cut.
fn line(v: &str, max: usize) -> String {
    clip(&v.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// `search_threads`' answer: Google's list and, for the first [`SUMMARIZED`] threads, what
/// [`summary_plan`] found (`summaries[i]` is the answer for thread `i`, null when it failed).
pub fn shape_search(list: &Value, summaries: &[Value]) -> Value {
    let threads: Vec<Value> = list
        .get("threads")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut m = Map::new();
                    m.insert("id".into(), json!(s(t, "/id")));
                    let snippet = s(t, "/snippet");
                    if !snippet.is_empty() {
                        m.insert("snippet".into(), json!(line(snippet, SNIPPET_CHARS)));
                    }
                    if let Some(sum) = summaries.get(i).filter(|v| !v.is_null()) {
                        let msgs = sum.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
                        if let Some(first) = msgs.first() {
                            let p = first.get("payload").unwrap_or(&Value::Null);
                            m.insert("subject".into(), json!(line(header(p, "Subject"), ADDRESS_CHARS)));
                            m.insert("from".into(), json!(line(header(p, "From"), ADDRESS_CHARS)));
                        }
                        if let Some(last) = msgs.last() {
                            m.insert("last_date".into(), json!(line(header(last.get("payload").unwrap_or(&Value::Null), "Date"), 60)));
                        }
                        m.insert("messages".into(), json!(msgs.len()));
                    }
                    Value::Object(m)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = Map::new();
    out.insert("threads".into(), json!(threads));
    let next = s(list, "/nextPageToken");
    if !next.is_empty() {
        out.insert("next_page_token".into(), json!(next));
    }
    Value::Object(out)
}

/// Gmail's text parts are base64url; decode one, lossily.
fn decode(data: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data.trim_end_matches('='))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// HTML as plain text: tags dropped, the common entities decoded, `script` and `style` removed.
fn html_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        let lower: String = tail.chars().take(8).collect::<String>().to_ascii_lowercase();
        let skip_to = ["script", "style"].iter().find_map(|t| {
            lower.strip_prefix('<').filter(|l| l.starts_with(t)).and_then(|_| tail.to_ascii_lowercase().find(&format!("</{t}")))
        });
        if let Some(end) = skip_to {
            let after = &tail[end..];
            rest = after.find('>').map_or("", |i| &after[i + 1..]);
            continue;
        }
        let block = lower.starts_with("<br") || lower.starts_with("<p") || lower.starts_with("</p") || lower.starts_with("<div") || lower.starts_with("</div") || lower.starts_with("<li") || lower.starts_with("<tr");
        if block {
            out.push('\n');
        }
        rest = tail.find('>').map_or("", |i| &tail[i + 1..]);
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// The message's readable text: its `text/plain` parts, else its `text/html` parts as text.
fn body_text(payload: &Value) -> String {
    fn collect(part: &Value, mime: &str, out: &mut Vec<String>) {
        if part.get("filename").and_then(Value::as_str).is_some_and(|f| !f.is_empty()) {
            return;
        }
        if s(part, "/mimeType") == mime {
            let data = s(part, "/body/data");
            if !data.is_empty() {
                out.push(decode(data));
            }
        }
        if let Some(parts) = part.get("parts").and_then(Value::as_array) {
            for p in parts {
                collect(p, mime, out);
            }
        }
    }
    let mut plain = Vec::new();
    collect(payload, "text/plain", &mut plain);
    if !plain.is_empty() {
        return plain.join("\n");
    }
    let mut html = Vec::new();
    collect(payload, "text/html", &mut html);
    html_text(&html.join("\n"))
}

/// Blank lines collapsed to one and the ends trimmed.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for l in text.replace('\r', "").lines() {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

/// A message's attachments: name, type and size. Their bytes are not fetched.
fn attachments(payload: &Value) -> Vec<Value> {
    fn walk(part: &Value, out: &mut Vec<Value>) {
        let name = s(part, "/filename");
        if !name.is_empty() && out.len() < ATTACHMENTS {
            out.push(json!({ "filename": line(name, 200), "mime_type": s(part, "/mimeType"), "size": part.pointer("/body/size").and_then(Value::as_u64).unwrap_or(0) }));
        }
        if let Some(parts) = part.get("parts").and_then(Value::as_array) {
            for p in parts {
                walk(p, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(payload, &mut out);
    out
}

fn shape_message(m: &Value, budget: &mut usize) -> Value {
    let p = m.get("payload").unwrap_or(&Value::Null);
    let mut out = Map::new();
    out.insert("id".into(), json!(s(m, "/id")));
    for (h, k) in [("From", "from"), ("To", "to"), ("Cc", "cc"), ("Subject", "subject"), ("Date", "date")] {
        let v = header(p, h);
        if !v.is_empty() {
            out.insert(k.into(), json!(line(v, ADDRESS_CHARS)));
        }
    }
    if let Some(l) = m.get("labelIds").and_then(Value::as_array) {
        out.insert("labels".into(), json!(l.iter().filter_map(Value::as_str).collect::<Vec<_>>()));
    }
    let text = tidy(&body_text(p));
    let room = BODY_CHARS.min(*budget);
    let shown = clip(&text, room);
    *budget = budget.saturating_sub(shown.chars().count());
    if shown.chars().count() < text.chars().count() {
        out.insert("body_cut".into(), json!(true));
    }
    out.insert("body".into(), json!(shown));
    let files = attachments(p);
    if !files.is_empty() {
        out.insert("attachments".into(), json!(files));
    }
    Value::Object(out)
}

/// Google's answer to a read action, trimmed for the agent.
pub fn shape(action: &str, body: &Value) -> Value {
    let mut out = Map::new();
    match action {
        "search_threads" => return shape_search(body, &[]),
        "get_thread" => {
            let all = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut budget = THREAD_CHARS;
            let shown: Vec<Value> = all.iter().take(MESSAGES).map(|m| shape_message(m, &mut budget)).collect();
            out.insert("id".into(), json!(s(body, "/id")));
            out.insert("messages".into(), json!(shown));
            if all.len() > MESSAGES {
                out.insert("messages_not_shown".into(), json!(all.len() - MESSAGES));
            }
        }
        "list_labels" => {
            let labels: Vec<Value> = body
                .get("labels")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|l| json!({ "id": s(l, "/id"), "name": line(s(l, "/name"), 200), "type": s(l, "/type") })).collect())
                .unwrap_or_default();
            out.insert("labels".into(), json!(labels));
        }
        _ => {}
    }
    Value::Object(out)
}

/// The ids a read touched, for the account's log (D7): never the search words or the mail.
pub fn logged_ids(action: &str, args: &GmailArgs) -> String {
    match action {
        "get_thread" => args.thread_id.as_deref().map(str::trim).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "gmail_tests.rs"]
mod tests;
