// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The action router behind a family tool (one tool with an `action`,
//! docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.1). Each action
//! names the internal handler it runs, by that handler's old tool name, and
//! the parameters the family tool calls differently, so one handler serves the
//! action unchanged.

use super::*;

/// One action of a family tool.
#[derive(Debug)]
pub(super) struct Act {
    pub(super) action: &'static str,
    /// The internal tool name its handler answers to.
    pub(super) tool: &'static str,
    /// `(family name, handler name)` for parameters named differently.
    pub(super) rename: &'static [(&'static str, &'static str)],
}

/// The action `arguments` asks for, with its arguments as the handler takes
/// them: `action` removed, renamed parameters under their handler names. An
/// unknown or missing action is an error that lists the actions.
pub(super) fn pick<'a>(family: &str, acts: &'a [Act], arguments: &Value) -> Result<(&'a Act, Value)> {
    let names = || acts.iter().map(|a| a.action).collect::<Vec<_>>().join(", ");
    let action = arguments.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if action.is_empty() {
        anyhow::bail!("{family}: missing required parameter: action (one of {})", names());
    }
    let Some(act) = acts.iter().find(|a| a.action == action) else {
        anyhow::bail!("{family}: unknown action {action:?}: use one of {}", names());
    };
    let mut args = match arguments {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    args.remove("action");
    for (ours, theirs) in act.rename {
        if let Some(v) = args.remove(*ours) {
            args.insert((*theirs).to_string(), v);
        }
    }
    Ok((act, Value::Object(args)))
}

/// `text` with each action's internal tool name, as a whole word, replaced by
/// the public one ("BrowserClick" → "Browser click"): handlers name the tool
/// they ran as in their output and errors, and that name isn't advertised.
pub(super) fn relabel(family: &str, acts: &[Act], text: &str) -> String {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = text.to_string();
    for act in acts {
        if !out.contains(act.tool) {
            continue;
        }
        let public = format!("{family} {}", act.action);
        let bytes = out.as_bytes();
        let mut next = String::with_capacity(out.len());
        let mut last = 0;
        for (i, _) in out.match_indices(act.tool) {
            let end = i + act.tool.len();
            let alone = (i == 0 || !word(bytes[i - 1])) && (end == bytes.len() || !word(bytes[end]));
            if alone {
                next.push_str(&out[last..i]);
                next.push_str(&public);
                last = end;
            }
        }
        next.push_str(&out[last..]);
        out = next;
    }
    out
}

/// A handler's result, output and error alike, under the public names.
pub(super) fn relabeled(family: &str, acts: &[Act], result: Result<String>) -> Result<String> {
    match result {
        Ok(text) => Ok(relabel(family, acts, &text)),
        Err(e) => Err(anyhow::anyhow!(relabel(family, acts, &format!("{e:#}")))),
    }
}

/// The actions' names, for a schema test.
#[cfg(test)]
pub(super) fn action_names(acts: &[Act]) -> Vec<&'static str> {
    acts.iter().map(|a| a.action).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTS: &[Act] = &[
        Act { action: "fill", tool: "BrowserFill", rename: &[] },
        Act { action: "wait", tool: "BrowserWaitFor", rename: &[("until_text", "text"), ("until_url", "url_contains")] },
    ];

    #[test]
    fn an_action_runs_its_handler_with_renamed_arguments() {
        let (act, args) = pick("Browser", ACTS, &json!({ "action": "wait", "until_text": "Done", "pane": "p1" })).unwrap();
        assert_eq!(act.tool, "BrowserWaitFor");
        assert_eq!(args, json!({ "text": "Done", "pane": "p1" }));
        let (act, args) = pick("Browser", ACTS, &json!({ "action": "fill", "ref": "e3", "text": "x" })).unwrap();
        assert_eq!(act.tool, "BrowserFill");
        assert_eq!(args, json!({ "ref": "e3", "text": "x" }));
    }

    #[test]
    fn handler_output_names_the_action_not_the_internal_tool() {
        assert_eq!(relabel("Browser", ACTS, "BrowserFill e12: done"), "Browser fill e12: done");
        assert_eq!(relabel("Browser", ACTS, "BrowserWaitFor failed: HTTP 504"), "Browser wait failed: HTTP 504");
        // Only whole words: a longer name that contains one is left alone.
        assert_eq!(relabel("Browser", ACTS, "XBrowserFill BrowserFillAll"), "XBrowserFill BrowserFillAll");
        let e = relabeled("Browser", ACTS, Err(anyhow::anyhow!("BrowserFill e3 failed"))).unwrap_err();
        assert_eq!(e.to_string(), "Browser fill e3 failed");
    }

    #[test]
    fn a_missing_or_unknown_action_lists_the_actions() {
        let e = pick("Browser", ACTS, &json!({})).unwrap_err().to_string();
        assert_eq!(e, "Browser: missing required parameter: action (one of fill, wait)");
        let e = pick("Browser", ACTS, &json!({ "action": "type" })).unwrap_err().to_string();
        assert_eq!(e, "Browser: unknown action \"type\": use one of fill, wait");
    }
}
