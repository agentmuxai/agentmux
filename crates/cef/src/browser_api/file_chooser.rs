// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `set_files` on an element that is not itself a file input: a button or label
//! the page wires to a hidden `<input type=file>`, or one that makes the input
//! when it is clicked (Apple's developer upload page).
//!
//! 1. A hidden input the element is tied to (a label's `for` or its content, an
//!    `aria-controls`, the one file input in the same form or control) is
//!    filled directly.
//! 2. Otherwise the element is clicked with the page's file chooser held (CDP
//!    `Page.setInterceptFileChooserDialog`: the browser shows no dialog and
//!    reports `Page.fileChooserOpened` with the input's node), and the files go
//!    into that input. The hold is on only for this call and every exit turns
//!    it off, so the person's own file dialog is never suppressed otherwise.
//!
//! The page's `accept` and `multiple` are honoured. The files are bytes srv read
//! from the agent's workspace; the page gets those, never a path.

use std::time::Duration;

use serde_json::{json, Value};

use super::act::{
    call_on, check_click, click_element, isolated_context, resolve, ClickCheck, SetFilesReq, FILES_READ_BACK, SET_FILES,
};
use super::cdp::{subscribe, CdpSession};
use super::snapshot::DomFacts;

/// How long a click may take to open the chooser.
const CHOOSER_WAIT: Duration = Duration::from_secs(5);

/// The hidden file input an element is tied to, as an object id, or `null`.
/// Looked for from the element outward: itself, its label, what it controls,
/// and the one file input in the nearest enclosing form or control.
const FIND_HIDDEN_INPUT: &str = r#"function () {
  const isFile = (e) => !!e && e.tagName === "INPUT" && String(e.type).toLowerCase() === "file";
  if (isFile(this)) return this;
  const doc = this.ownerDocument;
  const label = this.tagName === "LABEL" ? this : this.closest("label");
  if (label) {
    if (isFile(label.control)) return label.control;
    const inner = label.querySelector("input[type=file]");
    if (inner) return inner;
  }
  for (const id of (this.getAttribute("aria-controls") || "").split(/\s+/)) {
    const e = id && doc.getElementById(id);
    if (isFile(e)) return e;
  }
  for (let n = this.parentElement, depth = 0; n && depth < 6; n = n.parentElement, depth++) {
    const found = n.querySelectorAll("input[type=file]");
    if (found.length === 1) return found[0];
    if (found.length > 1 || n.tagName === "FORM") break;
  }
  return null;
}"#;

/// What an input takes.
const INPUT_RULES: &str = r#"function () {
  return { multiple: !!this.multiple, accept: this.getAttribute("accept") || "", disabled: !!this.disabled };
}"#;

/// Whether a file with this name and type fits an `accept` attribute: extensions
/// (`.pdf`), type families (`image/*`) and exact types (`application/pdf`). An
/// empty list takes anything; a file whose type isn't known can't fail a type
/// entry (only its extension is checked against the extensions).
pub(super) fn accept_allows(accept: &str, name: &str, mime: &str) -> bool {
    let entries: Vec<String> = accept.split(',').map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()).collect();
    if entries.is_empty() {
        return true;
    }
    let name = name.to_ascii_lowercase();
    let mime = mime.trim().to_ascii_lowercase();
    entries.iter().any(|entry| {
        if let Some(ext) = entry.strip_prefix('.') {
            name.ends_with(&format!(".{ext}"))
        } else if let Some(family) = entry.strip_suffix("/*") {
            mime.is_empty() || mime.starts_with(&format!("{family}/"))
        } else {
            mime.is_empty() || mime == *entry
        }
    })
}

/// The refusal for files the input doesn't take, or `None`.
pub(super) fn refuse_files(req: &SetFilesReq, multiple: bool, accept: &str) -> Option<String> {
    if req.files.len() > 1 && !multiple {
        return Some("this file input takes one file".to_string());
    }
    let bad: Vec<&str> = req.files.iter().filter(|f| !accept_allows(accept, &f.name, &f.mime)).map(|f| f.name.as_str()).collect();
    if bad.is_empty() {
        None
    } else {
        Some(format!("the page accepts only {accept:?}: {} doesn't match", bad.join(", ")))
    }
}

/// Puts the files in the input `input` (an object id) and reads back what it holds.
async fn fill_input(cdp: &mut CdpSession, input: &str, req: &SetFilesReq) -> Result<Value, String> {
    let rules = call_on(cdp, input, INPUT_RULES, vec![]).await?;
    if rules.get("disabled").and_then(Value::as_bool) == Some(true) {
        return Err("the file input is disabled".to_string());
    }
    let accept = rules.get("accept").and_then(Value::as_str).unwrap_or("");
    if let Some(why) = refuse_files(req, rules.get("multiple").and_then(Value::as_bool).unwrap_or(false), accept) {
        return Err(why);
    }
    let files = serde_json::to_value(&req.files).map_err(|e| e.to_string())?;
    call_on(cdp, input, SET_FILES, vec![files]).await?;
    call_on(cdp, input, FILES_READ_BACK, vec![]).await
}

/// The file input an element is tied to without a click, as an object id.
async fn hidden_input(cdp: &mut CdpSession, obj: &str) -> Option<String> {
    let v = cdp
        .call("Runtime.callFunctionOn", json!({ "objectId": obj, "functionDeclaration": FIND_HIDDEN_INPUT, "returnByValue": false }))
        .await
        .ok()?;
    v.pointer("/result/objectId").and_then(Value::as_str).map(str::to_string)
}

/// Turns the interception off when dropped (the call may be cut short), or at
/// once with [`now`](Self::now).
struct ChooserOff(Option<CdpSession>);

impl ChooserOff {
    async fn now(mut self) {
        if let Some(mut s) = self.0.take() {
            let _ = s.call("Page.setInterceptFileChooserDialog", json!({ "enabled": false })).await;
        }
    }
}

impl Drop for ChooserOff {
    fn drop(&mut self) {
        if let Some(mut s) = self.0.take() {
            tokio::spawn(async move {
                let _ = s.call("Page.setInterceptFileChooserDialog", json!({ "enabled": false })).await;
            });
        }
    }
}

/// Fills the file input behind the element `obj` (reference `r`): the hidden
/// input it is tied to, else the one its click asks the page to choose from.
pub(super) async fn set_files_via_element(
    root: &mut CdpSession,
    obj: &str,
    facts: &DomFacts,
    page: &str,
    r: &str,
    req: &SetFilesReq,
) -> Result<Value, String> {
    let mut cdp = root.in_session(None);
    if let Some(input) = hidden_input(&mut cdp, obj).await {
        return fill_input(&mut cdp, &input, req).await;
    }
    // The click is checked like `act`'s: one that commits something isn't made here.
    match check_click(&mut cdp, obj, facts, "", page, None, false).await? {
        ClickCheck::Click => {}
        ClickCheck::Ask(_) => {
            return Err(format!(
                "{r} isn't a file input, and clicking it would submit or commit something, so it wasn't clicked: pick the control that opens the file chooser"
            ));
        }
    }
    let browser_id = root.browser_id().ok_or("the pane closed")?;
    let mut opened = subscribe(browser_id, "Page.fileChooserOpened");
    let _ = root.call("Page.enable", json!({})).await;
    root.call("Page.setInterceptFileChooserDialog", json!({ "enabled": true })).await.map_err(|e| e.to_string())?;
    let off = ChooserOff(Some(root.in_session(None)));
    let result = choose(root, &mut cdp, obj, &mut opened.rx, r, req).await;
    off.now().await;
    drop(opened);
    result
}

async fn choose(
    root: &mut CdpSession,
    cdp: &mut CdpSession,
    obj: &str,
    opened: &mut tokio::sync::mpsc::UnboundedReceiver<Value>,
    r: &str,
    req: &SetFilesReq,
) -> Result<Value, String> {
    click_element(cdp, root, None, obj).await?;
    let event = loop {
        match tokio::time::timeout(CHOOSER_WAIT, opened.recv()).await {
            Ok(Some(e)) if e.get("sessionId").is_none() => break e,
            Ok(Some(_)) => continue,
            Ok(None) => return Err("the pane closed before a file chooser opened".to_string()),
            Err(_) => {
                return Err(format!(
                    "clicking {r} didn't open a file chooser within {} seconds: it isn't an upload control (take a new snapshot, or find a \"file input\")",
                    CHOOSER_WAIT.as_secs()
                ))
            }
        }
    };
    let params = event.get("params").cloned().unwrap_or(Value::Null);
    let single = params.get("mode").and_then(Value::as_str) == Some("selectSingle");
    if single && req.files.len() > 1 {
        return Err("the page's file chooser takes one file".to_string());
    }
    let backend = params
        .get("backendNodeId")
        .and_then(Value::as_i64)
        .ok_or("the browser didn't say which file input the page opened: choose the file in the pane")?;
    let ctx = isolated_context(cdp, None).await?;
    let input = resolve(cdp, backend, ctx, r)
        .await
        .map_err(|_| "the page's file input is gone already, so the files couldn't be put in it: take a new snapshot and try again".to_string())?;
    fill_input(cdp, &input, req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_api::act::UploadFile;

    fn file(name: &str, mime: &str) -> UploadFile {
        UploadFile { name: name.to_string(), mime: mime.to_string(), data: String::new() }
    }

    fn req(files: Vec<UploadFile>) -> SetFilesReq {
        SetFilesReq { block_id: "b".into(), ref_: "e1".into(), files }
    }

    #[test]
    fn an_empty_accept_takes_anything() {
        assert!(accept_allows("", "a.exe", "application/x-msdownload"));
        assert!(accept_allows(" , ", "a.exe", ""));
    }

    #[test]
    fn extensions_match_case_insensitively_at_the_end_of_the_name() {
        assert!(accept_allows(".pdf, .TXT", "scan.PDF", ""));
        assert!(accept_allows(".pdf,.txt", "notes.txt", "text/plain"));
        assert!(!accept_allows(".pdf,.txt", "photo.png", "image/png"));
        assert!(!accept_allows(".pdf", "pdf", ""), "an extension needs its dot");
    }

    #[test]
    fn type_families_and_exact_types_match_the_files_type() {
        assert!(accept_allows("image/*", "x.bin", "image/png"));
        assert!(!accept_allows("image/*", "x.pdf", "application/pdf"));
        assert!(accept_allows("application/pdf", "x", "application/pdf"));
        assert!(!accept_allows("application/pdf", "x", "text/plain"));
    }

    #[test]
    fn a_file_of_unknown_type_cant_fail_a_type_entry() {
        assert!(accept_allows("image/*", "photo", ""));
        assert!(accept_allows("application/pdf", "doc", ""));
        assert!(!accept_allows(".pdf", "doc", ""), "but an extension entry still needs its extension");
    }

    #[test]
    fn several_files_need_a_multiple_input_and_each_must_fit_accept() {
        let two = req(vec![file("a.txt", ""), file("b.txt", "")]);
        assert_eq!(refuse_files(&two, false, "").as_deref(), Some("this file input takes one file"));
        assert_eq!(refuse_files(&two, true, ".txt"), None);
        let mixed = req(vec![file("a.txt", ""), file("b.png", "")]);
        let why = refuse_files(&mixed, true, ".txt").unwrap();
        assert!(why.contains("b.png") && !why.contains("a.txt"), "{why}");
    }
}
