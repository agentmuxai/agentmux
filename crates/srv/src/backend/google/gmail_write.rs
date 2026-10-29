// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The `Gmail` tool's write actions (G3), named after Google's Gmail MCP tools
//! (SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §5, §6):
//! `create_draft`, `update_draft`, `label_thread` (no prompt with a write grant),
//! `send_draft` and `trash_thread` (always ask the person).
//!
//! There is no raw send: an agent writes a draft, and sending is `send_draft`, which shows the
//! person the draft exactly as it will go. This module is the pure half, as [`super::gmail`] is for
//! reads: the mail an agent describes as a message Google takes ([`outgoing`], [`rfc822`]), the
//! requests, what the person is shown ([`DraftView`], [`send_question`]) and the check that what
//! they approved is what is sent ([`DraftView::digest`]). The flow that asks and calls is
//! [`super::gmail_flow`].

use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::gmail::{body_text, get, header, id_arg, line, s, tidy, url_of, GmailArgs};
use super::{clip, GoogleRequest};

/// The write actions, in the order the tool lists them.
pub const WRITE_ACTIONS: [&str; 5] = ["create_draft", "update_draft", "send_draft", "label_thread", "trash_thread"];

/// Actions that always wait for the person (§6).
pub fn always_asks(action: &str) -> bool {
    matches!(action, "send_draft" | "trash_thread")
}

/// Most recipients, subject characters and body characters of one mail.
const RECIPIENTS: usize = 50;
const SUBJECT_CHARS: usize = 300;
const BODY_CHARS: usize = 100_000;
/// Label ids one `label_thread` adds or removes.
const LABELS: usize = 10;
/// How much of a mail's text the approval window shows.
const SHOWN_BODY_CHARS: usize = 3000;

/// The scopes an action can run with: any one is enough. A draft or a send is `gmail.compose`;
/// labels and trash need `gmail.modify` (which includes compose).
pub fn needs(action: &str) -> &'static [&'static str] {
    match action {
        "label_thread" | "trash_thread" => &["gmail.modify"],
        _ => &["gmail.compose", "gmail.modify"],
    }
}

/// Whether `granted` (full scope URLs, or bare names) covers the write `action`.
pub fn granted_for(action: &str, granted: &[String]) -> bool {
    needs(action).iter().any(|n| super::gmail::has_scope(granted, n))
}

/// One mail as an agent describes it, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// An address an agent gave: `name@example.com`, or `Name <name@example.com>` with a plain name. No
/// line break, no second address, nothing a mail header could be bent with.
fn address(raw: &str) -> Result<String, String> {
    let a = raw.trim();
    let bad = || format!("\"{}\" isn't an address I can send to; write it as name@example.com", clip(&line(a, 80), 60));
    if a.is_empty() || a.len() > 320 || a.chars().any(|c| c.is_control()) {
        return Err(bad());
    }
    let (name, email) = match (a.rfind('<'), a.ends_with('>')) {
        (Some(open), true) => (a[..open].trim(), &a[open + 1..a.len() - 1]),
        _ => ("", a),
    };
    let plain_name = name.chars().all(|c| c.is_ascii_alphanumeric() || " .'-_".contains(c));
    let (local, domain) = email.split_once('@').ok_or_else(bad)?;
    let part_ok = |p: &str, extra: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c));
    if !plain_name || !part_ok(local, ".+-_") || !part_ok(domain, ".-") || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') || email.len() > 254 {
        return Err(bad());
    }
    Ok(if name.is_empty() { email.to_string() } else { format!("{name} <{email}>") })
}

fn addresses(list: &Option<Vec<String>>) -> Result<Vec<String>, String> {
    list.iter().flatten().filter(|s| !s.trim().is_empty()).map(|a| address(a)).collect()
}

/// The mail `args` describes. A draft needs a recipient only in the end: `create_draft` may leave
/// `to` empty for the person to fill in, but nothing is sent without one.
pub fn outgoing(args: &GmailArgs) -> Result<Outgoing, String> {
    let (to, cc, bcc) = (addresses(&args.to)?, addresses(&args.cc)?, addresses(&args.bcc)?);
    if to.len() + cc.len() + bcc.len() > RECIPIENTS {
        return Err(format!("a mail goes to at most {RECIPIENTS} people"));
    }
    let subject = args.subject.as_deref().unwrap_or("").trim().to_string();
    if subject.chars().any(|c| c.is_control()) {
        return Err("the subject is a single line".to_string());
    }
    if subject.chars().count() > SUBJECT_CHARS {
        return Err(format!("the subject is at most {SUBJECT_CHARS} characters"));
    }
    let body = args.body.as_deref().unwrap_or("").to_string();
    if body.chars().count() > BODY_CHARS {
        return Err(format!("the body is at most {BODY_CHARS} characters"));
    }
    if subject.is_empty() && body.trim().is_empty() && to.is_empty() {
        return Err("a draft needs at least a recipient (to), a subject or a body".to_string());
    }
    Ok(Outgoing { to, cc, bcc, subject, body })
}

fn b64_lines(bytes: &[u8]) -> String {
    let all = base64::engine::general_purpose::STANDARD.encode(bytes);
    all.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n")
}

/// A subject as a header value: as is when ASCII, else RFC 2047 encoded words.
fn subject_header(subject: &str) -> String {
    if subject.is_ascii() {
        return subject.to_string();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in subject.chars() {
        if chunk.len() + c.len_utf8() > 36 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words.iter().map(|w| format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(w))).collect::<Vec<_>>().join("\r\n ")
}

/// The mail as RFC 5322 text, which Gmail takes (base64url) as a draft's `raw`. No `From`: Gmail
/// writes the account's own address.
pub fn rfc822(o: &Outgoing) -> String {
    let mut out = String::from("MIME-Version: 1.0\r\n");
    for (name, list) in [("To", &o.to), ("Cc", &o.cc), ("Bcc", &o.bcc)] {
        if !list.is_empty() {
            out.push_str(&format!("{name}: {}\r\n", list.join(", ")));
        }
    }
    out.push_str(&format!("Subject: {}\r\n", subject_header(&o.subject)));
    out.push_str("Content-Type: text/plain; charset=\"UTF-8\"\r\nContent-Transfer-Encoding: base64\r\n\r\n");
    out.push_str(&b64_lines(o.body.as_bytes()));
    out.push_str("\r\n");
    out
}

fn draft_body(o: &Outgoing, thread_id: Option<&str>) -> Value {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rfc822(o));
    let mut message = json!({ "raw": raw });
    if let Some(t) = thread_id {
        message["threadId"] = json!(t);
    }
    json!({ "message": message })
}

fn post(url: String, body: Value) -> GoogleRequest {
    GoogleRequest { method: reqwest::Method::POST, url, query: Vec::new(), body: Some(body), text: false }
}

/// The request for `create_draft` (or, with a draft id, `update_draft`). A `thread_id` makes the
/// draft a reply in that thread.
pub fn draft_plan(action: &str, args: &GmailArgs) -> Result<GoogleRequest, String> {
    let o = outgoing(args)?;
    let thread = match args.thread_id.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => Some(id_arg("thread_id", Some(t))?),
        None => None,
    };
    let body = draft_body(&o, thread.as_deref());
    match action {
        "create_draft" => Ok(post(url_of(&["drafts"])?, body)),
        "update_draft" => {
            let id = id_arg("draft_id", args.draft_id.as_deref())?;
            Ok(GoogleRequest { method: reqwest::Method::PUT, url: url_of(&["drafts", &id])?, query: Vec::new(), body: Some(body), text: false })
        }
        other => Err(format!("{other} doesn't write a draft")),
    }
}

/// The request for a draft as it is now, to show the person and to check before sending.
pub fn draft_fetch_plan(args: &GmailArgs) -> Result<GoogleRequest, String> {
    let id = id_arg("draft_id", args.draft_id.as_deref())?;
    Ok(get(url_of(&["drafts", &id])?, vec![("format", "full".to_string())]))
}

/// The request that sends the draft `id`.
pub fn send_plan(id: &str) -> Result<GoogleRequest, String> {
    let id = id_arg("draft_id", Some(id))?;
    Ok(post(url_of(&["drafts", "send"])?, json!({ "id": id })))
}

/// The request that moves a thread to the trash (it stays there for 30 days).
pub fn trash_plan(args: &GmailArgs) -> Result<GoogleRequest, String> {
    let id = id_arg("thread_id", args.thread_id.as_deref())?;
    Ok(post(url_of(&["threads", &id, "trash"])?, json!({})))
}

fn label_ids(name: &str, list: &Option<Vec<String>>) -> Result<Vec<String>, String> {
    let ids: Vec<String> = list.iter().flatten().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    if ids.len() > LABELS {
        return Err(format!("{name} takes at most {LABELS} label ids"));
    }
    for id in &ids {
        id_arg(name, Some(id)).map_err(|_| format!("\"{}\" isn't a label id; list_labels has them", clip(id, 40)))?;
    }
    Ok(ids)
}

/// The request for `label_thread`. Putting a thread in the trash takes `trash_thread`, which asks
/// the person, so the trash label is not one this can add.
pub fn label_plan(args: &GmailArgs) -> Result<GoogleRequest, String> {
    let thread = id_arg("thread_id", args.thread_id.as_deref())?;
    let add = label_ids("add_label_ids", &args.add_label_ids)?;
    let remove = label_ids("remove_label_ids", &args.remove_label_ids)?;
    if add.is_empty() && remove.is_empty() {
        return Err("label_thread needs add_label_ids or remove_label_ids (ids from list_labels)".to_string());
    }
    if add.iter().any(|l| l == "TRASH") {
        return Err("a thread goes to the trash with trash_thread, which asks the person first".to_string());
    }
    Ok(post(url_of(&["threads", &thread, "modify"])?, json!({ "addLabelIds": add, "removeLabelIds": remove })))
}

/// A draft as it is stored: what `send_draft` shows the person and checks again before sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftView {
    pub id: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
}

impl DraftView {
    /// From Google's `drafts.get` answer (format full).
    pub fn of(draft: &Value) -> DraftView {
        let msg = draft.get("message").unwrap_or(&Value::Null);
        let p = msg.get("payload").unwrap_or(&Value::Null);
        let h = |name: &str| header(p, name).split_whitespace().collect::<Vec<_>>().join(" ");
        DraftView { id: s(draft, "/id").to_string(), to: h("To"), cc: h("Cc"), bcc: h("Bcc"), subject: h("Subject"), body: tidy(&body_text(p)) }
    }

    /// A fingerprint of everything the person is asked about, so a draft changed after they said yes
    /// is not sent.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        for part in [&self.to, &self.cc, &self.bcc, &self.subject, &self.body] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part.as_bytes());
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Whether there is anyone to send it to.
    pub fn has_recipient(&self) -> bool {
        [&self.to, &self.cc, &self.bcc].iter().any(|r| !r.trim().is_empty())
    }
}

/// The title and text of the window that asks the person to send `view`.
pub fn send_question(agent: &str, account: &str, view: &DraftView) -> (String, String) {
    let shown = clip(&view.body, SHOWN_BODY_CHARS);
    let field = |name: &str, v: &str| if v.is_empty() { String::new() } else { format!("{name}: {v}\n") };
    let message = format!(
        "{agent} wants to send this email from {account}. It can't be taken back once it is sent.\n\n{}{}{}Subject: {}\n\n{shown}",
        field("To", &view.to),
        field("Cc", &view.cc),
        field("Bcc", &view.bcc),
        if view.subject.is_empty() { "(none)" } else { &view.subject },
    );
    (format!("{} wants to send an email", clip(&line(agent, 60), 60)), message)
}

/// The title and text of the window that asks the person to trash a thread. `summary` is the
/// thread's headers as [`super::gmail::summary_plan`] asks for them.
pub fn trash_question(agent: &str, account: &str, summary: &Value) -> (String, String) {
    let first = summary.get("messages").and_then(Value::as_array).and_then(|m| m.first());
    let p = first.and_then(|m| m.get("payload")).unwrap_or(&Value::Null);
    let count = summary.get("messages").and_then(Value::as_array).map_or(0, Vec::len);
    let message = format!(
        "{agent} wants to move this email thread in {account} to the trash (Gmail keeps it there for 30 days).\n\nSubject: {}\nFrom: {}\nMessages: {count}",
        clip(&line(header(p, "Subject"), 200), 200),
        clip(&line(header(p, "From"), 200), 200),
    );
    (format!("{} wants to trash an email", clip(&line(agent, 60), 60)), message)
}

#[cfg(test)]
#[path = "gmail_write_tests.rs"]
mod tests;
