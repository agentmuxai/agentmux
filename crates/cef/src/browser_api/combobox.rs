// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `select` on a custom combobox (an element with `role=combobox`, not a
//! native `<select>`): open it, find the option anywhere in the frame's
//! document (Angular CDK, Ant Design and others render the options in a portal
//! at the end of `<body>`, not under the control), click it, and read the
//! control back. With no matching option, type the text into the control and
//! choose the first match with ArrowDown and Enter, the ARIA authoring
//! practices keyboard pattern
//! (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §4.5).

use std::time::Duration;

use serde_json::{json, Value};

use super::act::{call_on, click_element, insert_text, refuse_typing_in_frame, secret_refusal, PREPARE_FILL};
use super::cdp::CdpSession;
use super::frames::FrameRef;
use super::key_typing;
use super::snapshot::{self, DomFacts};

/// Whether the element is a custom combobox: `role=combobox`, or a control
/// that says it pops up a listbox.
const IS_COMBOBOX: &str = r#"function () {
  const role = (this.getAttribute("role") || "").toLowerCase();
  const pop = (this.getAttribute("aria-haspopup") || "").toLowerCase();
  return { combo: role === "combobox" || pop === "listbox" };
}"#;

/// Shared by the two option searches: the visible `role=option` elements,
/// from the listbox the control names (`aria-controls`, `aria-owns`) when it
/// names one that has options, else from the whole document.
const OPTIONS_JS: &str = r#"const doc = this.ownerDocument;
  const visible = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  const within = (root) => Array.from(root.querySelectorAll('[role="option"]')).concat(root.matches('[role="option"]') ? [root] : []).filter(visible);
  const ids = ((this.getAttribute("aria-controls") || "") + " " + (this.getAttribute("aria-owns") || "")).split(/\s+/).filter(Boolean);
  let options = ids.map(i => doc.getElementById(i)).filter(Boolean).flatMap(within);
  if (!options.length) options = within(doc);
  const label = (o) => (o.getAttribute("aria-label") || o.textContent || "").replace(/\s+/g, " ").trim();"#;

/// The option whose label (or `data-value`) is `want`, or null.
fn find_option_js() -> String {
    format!(
        r#"function (want) {{
  {OPTIONS_JS}
  const w = String(want).trim().toLowerCase();
  return options.find(o => label(o).toLowerCase() === w) || options.find(o => (o.getAttribute("data-value") || "").toLowerCase() === w) || null;
}}"#
    )
}

/// The labels of the visible options, for a message that says what there is.
fn list_options_js() -> String {
    format!(
        r#"function () {{
  {OPTIONS_JS}
  return {{ options: options.slice(0, 40).map(label) }};
}}"#
    )
}

/// The control's state and what it shows: the value of an input, else its text.
const COMBO_READ: &str = r#"function () {
  const out = { tag: this.tagName.toLowerCase(), role: "combobox", expanded: this.getAttribute("aria-expanded") === "true" };
  out.value = "value" in this && this.tagName !== "BUTTON" ? this.value : (this.textContent || "").replace(/\s+/g, " ").trim();
  return out;
}"#;

/// Whether `obj` is a custom combobox rather than something `select` can't use.
pub(super) async fn is_combobox(cdp: &mut CdpSession, obj: &str) -> bool {
    call_on(cdp, obj, IS_COMBOBOX, vec![])
        .await
        .ok()
        .and_then(|v| v.get("combo").and_then(|c| c.as_bool()))
        .unwrap_or(false)
}

async fn expanded(cdp: &mut CdpSession, obj: &str) -> bool {
    call_on(cdp, obj, COMBO_READ, vec![])
        .await
        .ok()
        .and_then(|v| v.get("expanded").and_then(|e| e.as_bool()))
        .unwrap_or(false)
}

/// The object id of the option element, if the document has a visible one
/// named `want`. Not by value: the caller clicks the element.
async fn find_option(cdp: &mut CdpSession, obj: &str, want: &str) -> Option<String> {
    let v = cdp
        .call(
            "Runtime.callFunctionOn",
            json!({
                "objectId": obj,
                "functionDeclaration": find_option_js(),
                "arguments": [{ "value": want }],
                "returnByValue": false,
            }),
        )
        .await
        .ok()?;
    v.pointer("/result/objectId").and_then(|o| o.as_str()).map(str::to_string)
}

async fn shown_options(cdp: &mut CdpSession, obj: &str) -> Value {
    call_on(cdp, obj, &list_options_js(), vec![])
        .await
        .ok()
        .and_then(|v| v.get("options").cloned())
        .unwrap_or(Value::Null)
}

/// Chooses `want` in the custom combobox `obj`.
pub(super) async fn select(
    cdp: &mut CdpSession,
    root: &mut CdpSession,
    frame: Option<&FrameRef>,
    frame_id: Option<&str>,
    obj: &str,
    facts: &DomFacts,
    want: &str,
) -> Result<Value, String> {
    if !expanded(cdp, obj).await {
        click_element(cdp, root, frame, obj).await?;
        for _ in 0..6 {
            if expanded(cdp, obj).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    if let Some(option) = find_option(cdp, obj, want).await {
        let disabled = call_on(cdp, &option, r#"function () { return { v: this.getAttribute("aria-disabled") === "true" }; }"#, vec![]).await?;
        if disabled.get("v").and_then(|v| v.as_bool()) == Some(true) {
            return Err("that option is disabled".to_string());
        }
        click_element(cdp, root, frame, &option).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        return call_on(cdp, obj, COMBO_READ, vec![]).await;
    }
    // Typing is where the keyboard pattern starts: a secret field never gets it.
    if let Some(why) = snapshot::secret_reason(facts) {
        return Err(secret_refusal(why));
    }
    if call_on(cdp, obj, PREPARE_FILL, vec![]).await.is_err() {
        return Err(format!("no option {want:?} in this combobox; options shown: {}", shown_options(cdp, obj).await));
    }
    refuse_typing_in_frame(cdp, frame_id).await?;
    insert_text(cdp, want).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Enter with no option to take would submit the form the field is in.
    let listed = shown_options(cdp, obj).await;
    if listed.as_array().map_or(true, |o| o.is_empty()) {
        return Err(format!("no option {want:?} in this combobox: typing it showed no options (nothing was chosen)"));
    }
    key_typing::press(cdp, "ArrowDown").await?;
    key_typing::press(cdp, "Enter").await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let after = call_on(cdp, obj, COMBO_READ, vec![]).await?;
    let shown = after.get("value").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    if !shown.contains(&want.trim().to_lowercase()) {
        return Err(format!(
            "no option {want:?} was shown, and choosing the first match of what was typed left the combobox at {shown:?}; options shown: {}",
            listed
        ));
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_option_searches_share_one_way_of_finding_options() {
        // Both scripts embed the same search, so a fix to one can't skip the other.
        assert!(find_option_js().contains(OPTIONS_JS));
        assert!(list_options_js().contains(OPTIONS_JS));
    }

    #[test]
    fn the_option_search_reads_the_whole_document_not_only_the_control() {
        // A portal puts the options at the end of <body>, outside the control.
        assert!(OPTIONS_JS.contains("within(doc)"));
        assert!(OPTIONS_JS.contains("aria-controls"));
    }
}
