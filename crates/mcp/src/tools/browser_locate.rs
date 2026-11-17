// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` actions `locate` and `click_at`: the visual fallback for a region the page's accessibility
//! tree can't describe (a canvas UI, the picture of a button, a control with no accessible name). The
//! agent looks at a screenshot and names a point; the host finds what a click there would land on and gives
//! it a reference. `click_at` is that and then a click on the reference, so a committing click still asks
//! the person, the click must land on the element, and a frame off the pane's allowed sites is refused
//! (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md §4.6). There is no
//! `type_at`: to type, locate the field and fill its reference, so the secret guard always sees it.

use agentmux_common::api_types::UiBrowserLocateRequest;

use super::*;

/// What a locate found.
#[derive(Debug, PartialEq)]
struct Located {
    ref_: String,
    line: String,
    frame: Option<String>,
    secret: bool,
}

fn located(data: &Value) -> Option<Located> {
    Some(Located {
        ref_: data.get("ref")?.as_str()?.to_string(),
        line: data.get("line").and_then(Value::as_str).unwrap_or("").to_string(),
        frame: data.get("frame").and_then(Value::as_str).map(str::to_string),
        secret: data.get("secret").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// `x` or `y`, a position in the screenshot.
fn coordinate(arguments: &Value, key: &str) -> Result<f64> {
    arguments
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| anyhow::anyhow!("missing required parameter: {key} (a position in the screenshot in pixels, from its top left corner)"))
}

/// The element and where it is, as the agent reads it.
fn found_text(found: &Located, x: f64, y: f64) -> String {
    let frame = found.frame.as_deref().map(|f| format!(" in a frame from {f}")).unwrap_or_default();
    let secret = if found.secret { " It is a secret field: you can click it, but its value is never read or filled by you; hand off for that." } else { "" };
    format!("At ({x}, {y}) there is {}{frame} [ref={}].{secret}", found.line, found.ref_)
}

/// Ask the host what is at the point.
async fn locate(arguments: &Value, cx: &ToolCtx<'_>) -> Result<(Located, f64, f64)> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let (x, y) = (coordinate(arguments, "x")?, coordinate(arguments, "y")?);
    let image_width = arguments.get("image_width").and_then(Value::as_f64);
    let req_url = format!("{}/api/v1/ui/browser/locate", local_url.trim_end_matches('/'));
    let resp = client
        .post(&req_url)
        .header(AUTH_KEY_HEADER, auth_key)
        .json(&UiBrowserLocateRequest { auth, pane: pane_arg(arguments), x, y, image_width })
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("locate failed: HTTP {status} — {text}");
    }
    let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!("locate: bad response: {e}"))?;
    let found = located(body.get("data").unwrap_or(&Value::Null)).ok_or_else(|| anyhow::anyhow!("locate: the host found nothing at ({x}, {y})"))?;
    Ok((found, x, y))
}

pub(super) async fn run_locate(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let (found, x, y) = locate(arguments, cx).await?;
    Ok(format!(
        "{} Nothing was clicked: click it with Browser click, or fill, select or check it, with that ref. References last until the next snapshot or navigation.",
        found_text(&found, x, y)
    ))
}

pub(super) async fn run_click_at(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let (found, x, y) = locate(arguments, cx).await?;
    let click = json!({ "ref": found.ref_, "pane": arguments.get("pane") });
    let out = super::browser::run_act("BrowserClick", &click, cx).await?;
    // A click waiting for the person says so on its own; it is the answer as it is.
    if out.starts_with(super::browser_waiting::WAITING_PREFIX) {
        return Ok(out);
    }
    Ok(format!("{}\n{out}", found_text(&found, x, y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosts_answer_is_read_back_as_a_reference_and_a_line() {
        let d = json!({ "ref": "n1e1", "line": "button \"Next\"", "url": "https://x.example/", "secret": false });
        assert_eq!(located(&d), Some(Located { ref_: "n1e1".into(), line: "button \"Next\"".into(), frame: None, secret: false }));
        assert_eq!(located(&json!({})), None, "no reference, no element");
    }

    #[test]
    fn a_point_must_be_in_the_image() {
        assert_eq!(coordinate(&json!({ "x": 12.5 }), "x").unwrap(), 12.5);
        assert!(coordinate(&json!({ "x": -1 }), "x").is_err());
        assert!(coordinate(&json!({ "x": "7" }), "x").unwrap_err().to_string().contains("missing required parameter: x"));
        assert!(coordinate(&json!({}), "y").is_err());
    }

    #[test]
    fn the_answer_names_the_element_its_frame_and_its_reference() {
        let f = Located { ref_: "n2e1".into(), line: "textbox \"Card number\"".into(), frame: Some("https://pay.example".into()), secret: false };
        let t = found_text(&f, 40.0, 90.0);
        assert!(t.starts_with("At (40, 90) there is textbox \"Card number\" in a frame from https://pay.example [ref=n2e1]."), "{t}");
        let s = found_text(&Located { secret: true, frame: None, ..f }, 1.0, 2.0);
        assert!(s.contains("secret field"), "{s}");
        assert!(!s.contains("frame from"), "{s}");
    }
}
