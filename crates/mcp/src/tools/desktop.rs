// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Desktop: look at and control a native app, with the user's consent for
//! each app (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.1).
//!
//! The last rung of the ladder: connectors, an app's own API, `Browser`,
//! `Terminal` and `UI` come first. Every action but `apps` and `wait` asks
//! srv's gate (`/api/v1/agent/desktop/authorize`) first; srv answers from the
//! grants, the blocklist, the limits and Stop, and asks the user in the app
//! when there is no grant. The tool then acts (`desktop_input`) and tells srv
//! what happened, for the audit log.

use agentmux_common::api_types::{
    DesktopAuthorizeRequest, DesktopPolicyRequest, DesktopRecordRequest, DesktopReleaseRequest, DesktopWindowRef,
};

use super::*;
use crate::desktop_input::{self as input, Look, Rect};
use crate::window_capture::{
    capture_window_dir, enumerate_agentmux_windows, prune_old_captures, sha256_file, AgentMuxWindowInfo, CaptureTier,
};

/// Windows `apps` lists at most.
const MAX_LISTED: usize = 40;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Desktop" => {
            require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
            match str_arg(arguments, "action").as_deref() {
                Some("apps") => apps(cx).await,
                Some("look") => look(arguments, cx).await,
                Some("click") => click(arguments, cx).await,
                Some("scroll") => scroll(arguments, cx).await,
                Some("type") => type_text(arguments, cx).await,
                Some("key") => key(arguments, cx).await,
                Some("wait") => wait(arguments).await,
                Some("release") => release(arguments, cx).await,
                other => anyhow::bail!(
                    "Desktop: unknown action {other:?}; use apps, look, click, scroll, type, key, wait or release"
                ),
            }
        }
        _ => Err(not_in_family()),
    }
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn int_arg(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)))
}

fn window_id(args: &Value) -> Result<u32> {
    let id = int_arg(args, "window").ok_or_else(|| anyhow::anyhow!("window is required: the id `apps` listed"))?;
    u32::try_from(id).map_err(|_| anyhow::anyhow!("window {id} is not a window id from `apps`"))
}

/// The app's name as shown: the executable's file name without `.exe`.
fn app_label(exe_or_name: &str) -> String {
    let name = exe_or_name.rsplit(['\\', '/']).next().unwrap_or(exe_or_name);
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

fn window_ref(w: &AgentMuxWindowInfo, id: u32) -> DesktopWindowRef {
    let app = if w.exe_path.is_empty() { w.window.app_name().unwrap_or_default() } else { w.exe_path.clone() };
    DesktopWindowRef { id, app, title: w.title.clone() }
}

/// A window by the id `apps` gave. Only a foreign app of the user's own can
/// be driven (spec §5.3); the others are refused with where to go instead.
fn find_window(id: u32) -> Result<AgentMuxWindowInfo> {
    let w = enumerate_agentmux_windows()?
        .into_iter()
        .find(|w| w.window.id().ok() == Some(id))
        .ok_or_else(|| anyhow::anyhow!("window {id} is gone; call apps for the current list"))?;
    match w.tier {
        CaptureTier::ForeignApp => Ok(w),
        CaptureTier::SameInstance | CaptureTier::OtherInstance => {
            anyhow::bail!("that is AgentMux itself; Desktop doesn't drive it. Use the UI tool")
        }
        CaptureTier::OtherUser => anyhow::bail!("that window belongs to another user; it can't be controlled"),
    }
}

async fn post(cx: &ToolCtx<'_>, path: &str, body: &impl serde::Serialize, secs: u64) -> Result<Value> {
    let url = format!("{}{path}", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(body)
        .timeout(std::time::Duration::from_secs(secs))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let json: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        anyhow::bail!("Desktop: srv answered {status}: {}", json["error"].as_str().unwrap_or(&json.to_string()));
    }
    Ok(json)
}

/// Ask srv whether `action` on `w` may go ahead. Waits while the user is asked
/// (up to ten minutes). Returns the grant's id, or the reason as an error.
async fn authorize(cx: &ToolCtx<'_>, action: &str, w: &AgentMuxWindowInfo, id: u32) -> Result<String> {
    let body = DesktopAuthorizeRequest { auth: sign_ui_automation_auth()?, action: action.to_string(), window: window_ref(w, id) };
    let answer = post(cx, "/api/v1/agent/desktop/authorize", &body, 11 * 60).await?;
    match answer["status"].as_str() {
        Some("allow") => Ok(answer["grant_id"].as_str().unwrap_or("").to_string()),
        _ => anyhow::bail!("{}", answer["message"].as_str().unwrap_or("Desktop refused this")),
    }
}

/// Tell srv what an authorized action did. Best effort: the action has
/// happened (or failed) either way.
async fn record(cx: &ToolCtx<'_>, grant: &str, action: &str, w: &AgentMuxWindowInfo, id: u32, details: Value, result: &str) {
    let Ok(auth) = sign_ui_automation_auth() else { return };
    let body = DesktopRecordRequest {
        auth,
        grant_id: Some(grant.to_string()).filter(|g| !g.is_empty()),
        action: action.to_string(),
        window: window_ref(w, id),
        details,
        result: result.to_string(),
    };
    let _ = post(cx, "/api/v1/agent/desktop/record", &body, 10).await;
}

async fn apps(cx: &ToolCtx<'_>) -> Result<String> {
    let mut candidates: Vec<AgentMuxWindowInfo> = enumerate_agentmux_windows()?
        .into_iter()
        .filter(|w| w.tier == CaptureTier::ForeignApp && !w.title.trim().is_empty())
        .collect();
    candidates.truncate(MAX_LISTED);
    let refs: Vec<DesktopWindowRef> = candidates
        .iter()
        .filter_map(|w| w.window.id().ok().map(|id| window_ref(w, id)))
        .collect();
    let body = DesktopPolicyRequest { auth: sign_ui_automation_auth()?, windows: refs.clone() };
    let policy = post(cx, "/api/v1/agent/desktop/policy", &body, 30).await?;
    if policy["enabled"].as_bool() != Some(true) {
        return Ok("Desktop driving is off. The user turns it on in Settings (desktop:enabled); until they do, nothing is captured or controlled.".into());
    }
    let state_of = |id: u32| policy["windows"].as_array().and_then(|a| a.iter().find(|w| w["id"].as_u64() == Some(id as u64)));
    let mut lines = Vec::new();
    for r in &refs {
        let st = state_of(r.id);
        let app = app_label(&r.app);
        // A blocked window's title is not shown: it could be a password
        // manager's, and the listing is not the place to leak it.
        match st.and_then(|s| s["blocked"].as_str()) {
            Some(why) => lines.push(format!("{:>10}  {app} — blocked: {why}", r.id)),
            None => {
                let granted = st.map(|s| s["granted"].as_bool() == Some(true)).unwrap_or(false);
                let note = if granted { "you may drive it" } else { "the user is asked on first use" };
                let title: String = r.title.chars().take(80).collect();
                lines.push(format!("{:>10}  {app} — {title:?} — {note}", r.id));
            }
        }
    }
    if lines.is_empty() {
        return Ok("No app windows other than AgentMux are open.".into());
    }
    Ok(format!(
        "Windows you could control (pass the id as window). Use a connector, Browser, Terminal or the app's own API first; Desktop is for apps with no other way in.\n{}",
        lines.join("\n")
    ))
}

async fn look(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    input::win::ensure_dpi_aware();
    let w = find_window(id)?;
    let grant = authorize(cx, "look", &w, id).await?;
    // Look at the window as it is after the user answered.
    let w = find_window(id)?;
    let result = (|| -> Result<(String, Look, Option<String>)> {
        let rect = input::win::window_rect(id).ok_or_else(|| anyhow::anyhow!("couldn't read the window's size"))?;
        let image = w.window.capture_image().map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
        let (tw, th) = input::scaled_size(image.width(), image.height());
        let shown = if (tw, th) == (image.width(), image.height()) {
            image
        } else {
            image::imageops::resize(&image, tw, th, image::imageops::FilterType::Triangle)
        };
        let dir = capture_window_dir().map_err(|e| anyhow::anyhow!("cannot resolve the AgentMux root for captures: {e}"))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
        shown.save(&path).map_err(|e| anyhow::anyhow!("failed to save the capture: {e}"))?;
        prune_old_captures(&dir);
        let look = Look { rect, image_w: shown.width(), image_h: shown.height() };
        Ok((path.display().to_string(), look, sha256_file(&path)))
    })();
    match result {
        Ok((path, look, sha)) => {
            input::remember_look(id, look);
            record(cx, &grant, "look", &w, id, json!({ "image": [look.image_w, look.image_h], "sha256": sha }), "ok").await;
            Ok(format!(
                "Looked at {:?}: {path} — use Read on that path to see it. The image is {}×{} pixels; click, scroll and the other actions take x and y in the pixels of this image. Look again after anything changes, or if the window moves.",
                w.title, look.image_w, look.image_h
            ))
        }
        Err(e) => {
            record(cx, &grant, "look", &w, id, Value::Null, &format!("error: {e}")).await;
            Err(e)
        }
    }
}

/// The screen point for image point (x, y) of window `id`, from its last
/// look. Errors say what to do (look again, stay inside the image).
fn screen_point(id: u32, x: i64, y: i64) -> Result<(i32, i32)> {
    let look = input::last_look(id).ok_or_else(|| anyhow::anyhow!("look at the window first: x and y are pixels of that image"))?;
    let now: Rect = input::win::window_rect(id).ok_or_else(|| anyhow::anyhow!("the window is gone; call apps"))?;
    input::to_screen(&look, now, x, y).map_err(|e| anyhow::anyhow!("{e}"))
}

/// What every input action does after the gate: bring the window forward and
/// check the point isn't covered, run `act`, and record the outcome.
async fn drive(
    cx: &ToolCtx<'_>,
    action: &str,
    id: u32,
    point: Option<(i64, i64)>,
    details: Value,
    act: impl FnOnce(Option<(i32, i32)>) -> Result<(), String>,
) -> Result<AgentMuxWindowInfo> {
    input::win::ensure_dpi_aware();
    let w = find_window(id)?;
    // Check the point before asking, so a bad one doesn't use up a slot.
    if let Some((x, y)) = point {
        screen_point(id, x, y)?;
    }
    let grant = authorize(cx, action, &w, id).await?;
    // The user may have taken a while to answer: read the window again.
    let w = find_window(id)?;
    let outcome = (|| -> Result<(), String> {
        let screen = match point {
            Some((x, y)) => Some(screen_point(id, x, y).map_err(|e| e.to_string())?),
            None => None,
        };
        input::win::bring_to_front(id)?;
        if let Some((sx, sy)) = screen {
            input::win::point_belongs_to(id, sx, sy)?;
        }
        act(screen)
    })();
    match &outcome {
        Ok(()) => record(cx, &grant, action, &w, id, details, "ok").await,
        Err(e) => record(cx, &grant, action, &w, id, details, &format!("error: {e}")).await,
    }
    outcome.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(w)
}

async fn click(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let x = int_arg(args, "x").ok_or_else(|| anyhow::anyhow!("click needs x and y: pixels of your last look"))?;
    let y = int_arg(args, "y").ok_or_else(|| anyhow::anyhow!("click needs x and y: pixels of your last look"))?;
    let button = str_arg(args, "button").unwrap_or_else(|| "left".into());
    let count = int_arg(args, "count").unwrap_or(1).clamp(1, 3) as u32;
    let details = json!({ "x": x, "y": y, "button": button, "count": count });
    let b = button.clone();
    let w = drive(cx, "click", id, Some((x, y)), details, move |p| {
        let (sx, sy) = p.ok_or("no point")?;
        input::win::click(sx, sy, &b, count)
    })
    .await?;
    Ok(format!("Clicked ({x}, {y}) in {:?} ({button}{}). Look again to see the result.", w.title, if count > 1 { format!(" ×{count}") } else { String::new() }))
}

async fn scroll(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let x = int_arg(args, "x").ok_or_else(|| anyhow::anyhow!("scroll needs x and y: where in your last look"))?;
    let y = int_arg(args, "y").ok_or_else(|| anyhow::anyhow!("scroll needs x and y: where in your last look"))?;
    let dx = int_arg(args, "dx").unwrap_or(0).clamp(-50, 50) as i32;
    let dy = int_arg(args, "dy").unwrap_or(0).clamp(-50, 50) as i32;
    if dx == 0 && dy == 0 {
        anyhow::bail!("scroll needs dy (notches, positive is up) or dx (positive is right)");
    }
    let w = drive(cx, "scroll", id, Some((x, y)), json!({ "x": x, "y": y, "dx": dx, "dy": dy }), move |p| {
        let (sx, sy) = p.ok_or("no point")?;
        input::win::scroll(sx, sy, dx, dy)
    })
    .await?;
    Ok(format!("Scrolled at ({x}, {y}) in {:?} (dx {dx}, dy {dy}). Look again to see the result.", w.title))
}

async fn type_text(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    input::check_text(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let chars = text.chars().count();
    // The log gets the length, never the text.
    let w = drive(cx, "type", id, None, json!({ "chars": chars }), move |_| input::win::type_text(&text)).await?;
    Ok(format!("Typed {chars} characters into {:?}. Don't type passwords: ask the user to. Look again to see the result.", w.title))
}

async fn key(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let keys = str_arg(args, "keys").ok_or_else(|| anyhow::anyhow!("key needs keys, such as \"enter\" or \"ctrl+s\""))?;
    let chord = input::parse_keys(&keys).map_err(|e| anyhow::anyhow!("{e}"))?;
    let w = drive(cx, "key", id, None, json!({ "keys": keys }), move |_| input::win::press(&chord)).await?;
    Ok(format!("Pressed {keys} in {:?}. Look again to see the result.", w.title))
}

async fn wait(args: &Value) -> Result<String> {
    let ms = int_arg(args, "ms").ok_or_else(|| anyhow::anyhow!("wait needs ms"))?;
    let ms = ms.clamp(0, input::MAX_WAIT_MS as i64) as u64;
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    Ok(format!("Waited {ms} ms."))
}

async fn release(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let w = find_window(id)?;
    let body = DesktopReleaseRequest { auth: sign_ui_automation_auth()?, window: window_ref(&w, id) };
    let answer = post(cx, "/api/v1/agent/desktop/release", &body, 30).await?;
    Ok(if answer["released"].as_bool() == Some(true) {
        format!("Gave back your access to {}.", app_label(&w.exe_path))
    } else {
        "You had no access to give back.".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_is_shown_by_its_executable_name() {
        assert_eq!(app_label(r"C:\Windows\System32\notepad.exe"), "notepad");
        assert_eq!(app_label("/usr/bin/gedit"), "gedit");
        assert_eq!(app_label("calc"), "calc");
    }

    #[test]
    fn window_ids_are_integers_from_apps() {
        assert_eq!(window_id(&json!({ "window": 66052 })).unwrap(), 66052);
        assert!(window_id(&json!({})).is_err());
        assert!(window_id(&json!({ "window": -1 })).is_err());
        assert!(window_id(&json!({ "window": "x" })).is_err());
    }

    #[test]
    fn numbers_may_arrive_as_floats() {
        assert_eq!(int_arg(&json!({ "x": 10.0 }), "x"), Some(10));
        assert_eq!(int_arg(&json!({ "x": "10" }), "x"), None);
    }
}
