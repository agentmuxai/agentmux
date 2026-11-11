// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` action `zoom`: show a pane's page smaller or larger so what a click needs is on screen
//! (docs/specs/SPEC_BROWSER_PANE_AGENT_ZOOM_2026_11_10.md), and the line every Browser answer ends with while
//! a pane is zoomed.

use agentmux_common::api_types::UiBrowserZoomRequest;

use super::*;

/// What `zoom` was asked for.
#[derive(Debug, PartialEq)]
struct Ask {
    factor: Option<f64>,
    width: Option<f64>,
    fit: bool,
    reset: bool,
}

/// `factor` as a number, or a string like `"50%"` or `"0.5"`.
fn parse_factor(v: &Value) -> Result<f64> {
    let bad = || anyhow::anyhow!("factor must be a number from 0.25 to 2.0, or a percentage like \"50%\"");
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(bad),
        Value::String(s) => {
            let s = s.trim();
            match s.strip_suffix('%') {
                Some(p) => p.trim().parse::<f64>().map(|p| p / 100.0).map_err(|_| bad()),
                None => s.parse::<f64>().map_err(|_| bad()),
            }
        }
        _ => Err(bad()),
    }
}

fn parse_width(v: &Value) -> Result<f64> {
    let bad = || anyhow::anyhow!("width must be a number of CSS pixels, 320 to 3840");
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(bad),
        Value::String(s) => s.trim().parse::<f64>().map_err(|_| bad()),
        _ => Err(bad()),
    }
}

fn ask_from(arguments: &Value) -> Result<Ask> {
    let given = |k: &str| arguments.get(k).filter(|v| !v.is_null());
    let flag = |k: &str| given(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let ask = Ask {
        factor: given("factor").map(parse_factor).transpose()?,
        // `width: true` is a desktop viewport, 1280 CSS pixels wide.
        width: match given("width") {
            Some(Value::Bool(true)) => Some(1280.0),
            Some(Value::Bool(false)) | None => None,
            Some(v) => Some(parse_width(v)?),
        },
        fit: flag("fit"),
        reset: flag("reset"),
    };
    let asked = [ask.factor.is_some(), ask.width.is_some(), ask.fit, ask.reset].iter().filter(|a| **a).count();
    if asked != 1 {
        anyhow::bail!("zoom takes exactly one of factor (0.25 to 2.0, or \"50%\"), width (a desktop viewport in CSS pixels, e.g. 1280), fit or reset");
    }
    Ok(ask)
}

pub(super) async fn run_zoom(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let ask = ask_from(arguments)?;
    let req_url = format!("{}/api/v1/ui/browser/zoom", local_url.trim_end_matches('/'));
    let resp = client
        .post(&req_url)
        .header(AUTH_KEY_HEADER, auth_key)
        .json(&UiBrowserZoomRequest {
            auth,
            pane: pane_arg(arguments),
            factor: ask.factor,
            width: ask.width,
            fit: ask.fit.then_some(true),
            reset: ask.reset.then_some(true),
        })
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("zoom failed: HTTP {status} — {text}");
    }
    let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!("zoom: bad response: {e}"))?;
    Ok(match body.pointer("/data/line").and_then(|l| l.as_str()) {
        Some(line) if ask.reset => format!("Zoom reset: {line}"),
        Some(line) => format!("Zoomed: {line}. Refs and clicks work as before; a screenshot shows the page at this size."),
        None => "Zoomed".to_string(),
    })
}

/// The lines about this agent's zoomed panes that end a Browser answer; empty when none is zoomed.
pub(super) fn zooms_text(zooms: Option<&Value>) -> String {
    let Some(list) = zooms.and_then(|v| v.as_array()).filter(|l| !l.is_empty()) else {
        return String::new();
    };
    let mut out = String::from("\n\n");
    for z in list {
        let s = |k: &str| z.get(k).and_then(|v| v.as_str()).unwrap_or("");
        out.push_str(&format!("Zoom: pane {} is zoomed ({}); Browser zoom with reset puts it back.\n", s("pane"), s("line")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_factor_is_a_number_or_a_percentage() {
        assert_eq!(parse_factor(&json!(0.5)).unwrap(), 0.5);
        assert_eq!(parse_factor(&json!("50%")).unwrap(), 0.5);
        assert_eq!(parse_factor(&json!(" 125 % ")).unwrap(), 1.25);
        assert_eq!(parse_factor(&json!("0.75")).unwrap(), 0.75);
        assert!(parse_factor(&json!("half")).is_err());
        assert!(parse_factor(&json!(true)).is_err());
    }

    #[test]
    fn exactly_one_thing_is_asked_for() {
        let ask = ask_from(&json!({ "action": "zoom", "factor": "50%" })).unwrap();
        assert_eq!(ask, Ask { factor: Some(0.5), width: None, fit: false, reset: false });
        assert_eq!(ask_from(&json!({ "width": 1280 })).unwrap().width, Some(1280.0));
        assert_eq!(ask_from(&json!({ "width": true })).unwrap().width, Some(1280.0));
        assert!(ask_from(&json!({ "fit": true })).unwrap().fit);
        assert!(ask_from(&json!({ "reset": true })).unwrap().reset);
        assert!(ask_from(&json!({})).is_err(), "nothing asked");
        assert!(ask_from(&json!({ "factor": 0.5, "fit": true })).is_err(), "two things");
        assert!(ask_from(&json!({ "fit": false })).is_err(), "false asks for nothing");
    }

    #[test]
    fn the_note_names_each_zoomed_pane_and_is_empty_without_one() {
        assert_eq!(zooms_text(None), "");
        assert_eq!(zooms_text(Some(&json!([]))), "");
        let t = zooms_text(Some(&json!([{ "pane": "b1", "line": "viewport 1280x720 CSS, shown at 50%" }])));
        assert!(t.contains("pane b1 is zoomed (viewport 1280x720 CSS, shown at 50%)"), "{t}");
        assert!(t.contains("reset"), "{t}");
    }
}
