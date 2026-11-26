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
                Some(a @ ("click" | "scroll" | "drag" | "type" | "key")) => {
                    let said = match a {
                        "click" => click(arguments, cx).await?,
                        "scroll" => scroll(arguments, cx).await?,
                        "drag" => drag(arguments, cx).await?,
                        "type" => type_text(arguments, cx).await?,
                        _ => key(arguments, cx).await?,
                    };
                    Ok(with_look(arguments, cx, said).await)
                }
                Some("wait") => wait(arguments).await,
                Some("release") => release(arguments, cx).await,
                other => anyhow::bail!(
                    "Desktop: unknown action {other:?}; use apps, look, click, scroll, drag, type, key, wait or release"
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

/// The process that hosts every Windows Store (UWP) app's window.
const FRAME_HOST: &str = "ApplicationFrameHost";

/// Why a Windows Store app's window can't be used right now.
const UNIDENTIFIED_STORE_APP: &str =
    "this is a Windows Store app whose app can't be identified while its window is minimised; ask the user to restore it, then call apps again";

/// The window as srv names it. The app is what grants, denials and the
/// blocklist key on, so a Windows Store app is named by its own executable,
/// not by `ApplicationFrameHost.exe`, which hosts all of them: a grant for
/// Calculator must not cover every other Store app, and a Store password
/// manager must stay blockable by name. When the hosted app can't be found
/// (it is suspended while minimised), the window is refused, never keyed on
/// the frame host.
fn window_ref(w: &AgentMuxWindowInfo, id: u32) -> Result<DesktopWindowRef> {
    let app = if w.exe_path.is_empty() { w.window.app_name().unwrap_or_default() } else { w.exe_path.clone() };
    let app = if app_label(&app).eq_ignore_ascii_case(FRAME_HOST) {
        input::win::hosted_app_exe(id).ok_or_else(|| anyhow::anyhow!("{UNIDENTIFIED_STORE_APP}"))?
    } else {
        app
    };
    Ok(DesktopWindowRef { id, app, title: w.title.clone() })
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

/// Ask srv whether `action` on `w` may go ahead. Returns the grant's id, or
/// srv's reason as an error. It never waits for the user: with no grant srv
/// asks them and answers `pending` at once, and their answer reaches the agent
/// as an [AgentMux] note.
async fn authorize(cx: &ToolCtx<'_>, action: &str, w: &AgentMuxWindowInfo, id: u32) -> Result<String> {
    let body = DesktopAuthorizeRequest { auth: sign_ui_automation_auth()?, action: action.to_string(), window: window_ref(w, id)? };
    let answer = post(cx, "/api/v1/agent/desktop/authorize", &body, 30).await?;
    match answer["status"].as_str() {
        Some("allow") => Ok(answer["grant_id"].as_str().unwrap_or("").to_string()),
        _ => anyhow::bail!("{}", answer["message"].as_str().unwrap_or("Desktop refused this")),
    }
}

/// Tell srv what an authorized action did. Best effort: the action has
/// happened (or failed) either way.
async fn record(cx: &ToolCtx<'_>, grant: &str, action: &str, w: &AgentMuxWindowInfo, id: u32, details: Value, result: &str) {
    let Ok(auth) = sign_ui_automation_auth() else { return };
    // Authorized a moment ago, so the window was identified then.
    let Ok(window) = window_ref(w, id) else { return };
    let body = DesktopRecordRequest {
        auth,
        grant_id: Some(grant.to_string()).filter(|g| !g.is_empty()),
        action: action.to_string(),
        window,
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
    // A Store app that can't be identified is listed, not offered (window_ref).
    let mut refs: Vec<DesktopWindowRef> = Vec::new();
    let mut unidentified: Vec<u32> = Vec::new();
    for w in &candidates {
        let Ok(id) = w.window.id() else { continue };
        match window_ref(w, id) {
            Ok(r) => refs.push(r),
            Err(_) => unidentified.push(id),
        }
    }
    let body = DesktopPolicyRequest { auth: sign_ui_automation_auth()?, windows: refs.clone() };
    let policy = post(cx, "/api/v1/agent/desktop/policy", &body, 30).await?;
    if policy["enabled"].as_bool() != Some(true) {
        return Ok("Desktop driving is off. The user turns it on in Settings > Agents > Desktop driving; until they do, nothing is captured or controlled.".into());
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
                // A skill for this app, when the catalog has one. Reading it grants nothing.
                let skill = st.and_then(|s| s["skill"].as_object()).map(|k| {
                    let seen = if k.get("verified_here").and_then(Value::as_bool) == Some(true) { "tested here" } else { "not tested here" };
                    format!(" — skill: {} {} ({seen}); Skills get skill_id={}", k.get("name").and_then(Value::as_str).unwrap_or(""), k.get("version").and_then(Value::as_str).unwrap_or(""), k.get("id").and_then(Value::as_str).unwrap_or(""))
                });
                lines.push(format!("{:>10}  {app} — {title:?} — {note}{}", r.id, skill.unwrap_or_default()));
            }
        }
    }
    for id in unidentified {
        lines.push(format!("{id:>10}  a Windows Store app — minimised, so it can't be identified; it can be used once the user restores it"));
    }
    if lines.is_empty() {
        return Ok("No app windows other than AgentMux are open.".into());
    }
    Ok(format!(
        "Windows you could control (pass the id as window). Use a connector, Browser, Terminal or the app's own API first; Desktop is for apps with no other way in.\n{}",
        lines.join("\n")
    ))
}

/// `region` of a `look`: x, y, w, h in the pixels of the last look.
fn parse_region(args: &Value) -> Result<Option<(i64, i64, i64, i64)>> {
    let Some(r) = args.get("region").filter(|r| !r.is_null()) else { return Ok(None) };
    let g = |k: &str| int_arg(r, k).ok_or_else(|| anyhow::anyhow!("region needs x, y, w and h (pixels of your last look); {k} is missing"));
    Ok(Some((g("x")?, g("y")?, g("w")?, g("h")?)))
}

/// What a `look` made: the image's path and hash, and either the whole
/// window's look or the zoom of a region.
struct Captured {
    path: String,
    sha: Option<String>,
    kind: CaptureKind,
}

enum CaptureKind {
    Whole(Look),
    Zoom { of: Look, region: input::Region, zoom: f64, w: u32, h: u32 },
}

async fn look(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let region = parse_region(args)?;
    input::win::ensure_dpi_aware();
    let w = find_window(id)?;
    // A region is of the last look, so it needs one.
    let prior = input::last_look(id);
    if region.is_some() && prior.is_none() {
        anyhow::bail!("look at the whole window first: a region is in the pixels of that look");
    }
    let grant = authorize(cx, "look", &w, id).await?;
    // Look at the window as it is after the user answered.
    let w = find_window(id)?;
    let restored = input::win::restore_if_minimized(id);
    let result = (|| -> Result<Captured> {
        let rect = input::win::window_rect(id).ok_or_else(|| anyhow::anyhow!("couldn't read the window's size"))?;
        let image = w.window.capture_image().map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
        if image.width() == 0 || image.height() == 0 {
            anyhow::bail!("the window has no size to capture (it may be minimised or hidden); call look again, and ask the user to restore it if this repeats");
        }
        let dir = capture_window_dir().map_err(|e| anyhow::anyhow!("cannot resolve the AgentMux root for captures: {e}"))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
        let kind = if let Some((x, y, rw, rh)) = region {
            let of = prior.ok_or_else(|| anyhow::anyhow!("look at the whole window first"))?;
            if of.rect != rect {
                anyhow::bail!("the window moved or resized since look #{}; take a whole look first, then a region of it", of.n);
            }
            let r = input::check_region(&of, x, y, rw, rh).map_err(|e| anyhow::anyhow!("{e}"))?;
            // The look was scaled down from the capture: cut the same region from the capture itself.
            let (fx, fy) = (image.width() as f64 / of.image_w as f64, image.height() as f64 / of.image_h as f64);
            let (cx0, cy0) = ((r.x as f64 * fx).round() as u32, (r.y as f64 * fy).round() as u32);
            let cw = ((r.w as f64 * fx).round() as u32).clamp(1, image.width().saturating_sub(cx0).max(1));
            let ch = ((r.h as f64 * fy).round() as u32).clamp(1, image.height().saturating_sub(cy0).max(1));
            let cut = image::imageops::crop_imm(&image, cx0, cy0, cw, ch).to_image();
            let zoom = input::zoom_for(r.w, r.h);
            let (ow, oh) = (((r.w as f64 * zoom).round() as u32).max(1), ((r.h as f64 * zoom).round() as u32).max(1));
            let shown = image::imageops::resize(&cut, ow, oh, image::imageops::FilterType::CatmullRom);
            shown.save(&path).map_err(|e| anyhow::anyhow!("failed to save the capture: {e}"))?;
            CaptureKind::Zoom { of, region: r, zoom, w: ow, h: oh }
        } else {
            let (tw, th) = input::scaled_size(image.width(), image.height());
            let shown = if (tw, th) == (image.width(), image.height()) {
                image
            } else {
                image::imageops::resize(&image, tw, th, image::imageops::FilterType::Triangle)
            };
            shown.save(&path).map_err(|e| anyhow::anyhow!("failed to save the capture: {e}"))?;
            CaptureKind::Whole(Look { rect, image_w: shown.width(), image_h: shown.height(), n: 0 })
        };
        prune_old_captures(&dir);
        Ok(Captured { sha: sha256_file(&path), path: path.display().to_string(), kind })
    })();
    match result {
        Ok(c) => {
            let woke = if restored { " The window was minimised; it has been restored." } else { "" };
            let said = match c.kind {
                CaptureKind::Whole(look0) => {
                    let (look, before) = input::remember_look(id, look0);
                    record(cx, &grant, "look", &w, id, json!({ "image": [look.image_w, look.image_h], "look": look.n, "sha256": c.sha }), "ok").await;
                    format!(
                        "Looked at {:?}: {} — use Read on that path to see it. {} click, scroll, drag and the other actions take x and y in the pixels of this image (pass of_look {} to be told if they are from an older look). Look again after anything changes, or if the window moves.{woke}",
                        w.title,
                        c.path,
                        input::describe_scale(&look, before.as_ref()),
                        look.n
                    )
                }
                CaptureKind::Zoom { of, region: r, zoom, w: zw, h: zh } => {
                    record(cx, &grant, "look", &w, id, json!({ "region": [r.x, r.y, r.w, r.h], "of_look": of.n, "sha256": c.sha }), "ok").await;
                    format!(
                        "Looked at region ({}, {}, {}×{}) of look #{} in {:?}, enlarged ×{zoom:.1}: {} — use Read on that path to see it. The image is {zw}×{zh} pixels. It does not replace look #{}: keep clicking in look #{}'s pixels (a point (zx, zy) in this zoom is at x = {} + zx×{}/{zw}, y = {} + zy×{}/{zh} in look #{}).{woke}",
                        r.x, r.y, r.w, r.h, of.n, w.title, c.path, of.n, of.n, r.x, r.w, r.y, r.h, of.n
                    )
                }
            };
            Ok(said)
        }
        Err(e) => {
            record(cx, &grant, "look", &w, id, Value::Null, &format!("error: {e}")).await;
            Err(e)
        }
    }
}

/// After an action that asked for it (`look: true`), the fresh look at the
/// window, appended to what the action said: one call instead of three. A look
/// that fails doesn't fail the action, which has happened.
async fn with_look(args: &Value, cx: &ToolCtx<'_>, said: String) -> String {
    if args.get("look").and_then(Value::as_bool) != Some(true) {
        return said;
    }
    let Ok(id) = window_id(args) else { return said };
    // Give the app a moment to draw what the action did.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    match look(&json!({ "window": id }), cx).await {
        Ok(l) => format!("{said}\nThe window now: {l}"),
        Err(e) => format!("{said}\n(The look afterwards failed: {e})"),
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
/// check every point isn't covered, run `act` with the points' screen
/// positions, and record the outcome.
async fn drive(
    cx: &ToolCtx<'_>,
    action: &str,
    id: u32,
    points: &[(i64, i64)],
    of_look: Option<i64>,
    details: Value,
    act: impl FnOnce(&[(i32, i32)]) -> Result<(), String>,
) -> Result<AgentMuxWindowInfo> {
    input::win::ensure_dpi_aware();
    let w = find_window(id)?;
    // Coordinates the agent says are from an older look than the latest are stale.
    if let (Some(of), false) = (of_look, points.is_empty()) {
        let latest = input::last_look(id).ok_or_else(|| anyhow::anyhow!("look at the window first: x and y are pixels of that image"))?;
        input::check_of_look(&latest, of).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    // Check the points before asking, so a bad one doesn't use up a slot.
    for (x, y) in points {
        screen_point(id, *x, *y)?;
    }
    let grant = authorize(cx, action, &w, id).await?;
    // The user may have taken a while to answer: read the window again.
    let w = find_window(id)?;
    let outcome = (|| -> Result<(), String> {
        let screen = points
            .iter()
            .map(|(x, y)| screen_point(id, *x, *y).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        input::win::bring_to_front(id)?;
        for (sx, sy) in &screen {
            input::win::point_belongs_to(id, *sx, *sy)?;
        }
        act(&screen)
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
    let w = drive(cx, "click", id, &[(x, y)], int_arg(args, "of_look"), details, move |p| {
        let (sx, sy) = *p.first().ok_or("no point")?;
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
    let w = drive(cx, "scroll", id, &[(x, y)], int_arg(args, "of_look"), json!({ "x": x, "y": y, "dx": dx, "dy": dy }), move |p| {
        let (sx, sy) = *p.first().ok_or("no point")?;
        input::win::scroll(sx, sy, dx, dy)
    })
    .await?;
    Ok(format!("Scrolled at ({x}, {y}) in {:?} (dx {dx}, dy {dy}). Look again to see the result.", w.title))
}

/// A drag's two ends, the points it passes through, and what it holds, read
/// from the call's arguments.
#[derive(Debug, PartialEq, Eq)]
struct DragArgs {
    /// Where it presses, each `via` point in order, and where it lets go.
    points: Vec<(i64, i64)>,
    button: String,
    hold: Vec<u16>,
    hold_text: String,
}

/// Read a drag's arguments. Refuses what can't be a drag, with the fix.
fn parse_drag(args: &Value) -> Result<DragArgs> {
    let need = |k: &str| {
        int_arg(args, k).ok_or_else(|| {
            anyhow::anyhow!(
                "drag needs x, y (where to press) and x2, y2 (where to let go): pixels of your last look; {k} is missing"
            )
        })
    };
    let (x, y, x2, y2) = (need("x")?, need("y")?, need("x2")?, need("y2")?);
    let mut via = Vec::new();
    if let Some(v) = args.get("via") {
        let list = v.as_array().ok_or_else(|| anyhow::anyhow!("via is a list of {{x, y}} points to pass through"))?;
        if list.len() > input::MAX_DRAG_WAYPOINTS {
            anyhow::bail!("via has {} points; a drag passes through at most {}", list.len(), input::MAX_DRAG_WAYPOINTS);
        }
        for p in list {
            let (Some(vx), Some(vy)) = (int_arg(p, "x"), int_arg(p, "y")) else {
                anyhow::bail!("every via point needs x and y")
            };
            via.push((vx, vy));
        }
    }
    let mut points = vec![(x, y)];
    points.extend(via);
    points.push((x2, y2));
    if points.windows(2).all(|w| w[0] == w[1]) {
        anyhow::bail!("the drag starts and ends at the same point; use click for that");
    }
    let button = str_arg(args, "button").unwrap_or_else(|| "left".into());
    if !["left", "right", "middle"].contains(&button.as_str()) {
        anyhow::bail!("unknown button {button:?}; use left, right or middle");
    }
    let hold_text = str_arg(args, "hold").unwrap_or_default();
    let hold = input::parse_hold(&hold_text).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(DragArgs { points, button, hold, hold_text })
}

async fn drag(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let d = parse_drag(args)?;
    let (from, to) = (d.points[0], d.points[d.points.len() - 1]);
    let details = json!({
        "from": [from.0, from.1],
        "to": [to.0, to.1],
        "via": d.points.len() - 2,
        "button": d.button,
        "hold": d.hold_text,
    });
    let (button, hold) = (d.button.clone(), d.hold.clone());
    let w = drive(cx, "drag", id, &d.points, int_arg(args, "of_look"), details, move |screen| {
        let last = screen.len() - 1;
        input::win::drag(&input::drag_path(screen[0], &screen[1..last], screen[last]), &button, &hold)
    })
    .await?;
    let held = if d.hold_text.is_empty() { String::new() } else { format!(", holding {}", d.hold_text) };
    Ok(format!(
        "Dragged from ({}, {}) to ({}, {}) in {:?} ({}{held}); the button is let go. Look again to see the result.",
        from.0, from.1, to.0, to.1, w.title, d.button
    ))
}

async fn type_text(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    input::check_text(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    let chars = text.chars().count();
    // The log gets the length, never the text.
    let w = drive(cx, "type", id, &[], None, json!({ "chars": chars }), move |_| input::win::type_text(&text)).await?;
    Ok(format!("Typed {chars} characters into {:?}. Don't type passwords: ask the user to. Look again to see the result.", w.title))
}

async fn key(args: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let id = window_id(args)?;
    let keys = str_arg(args, "keys").ok_or_else(|| anyhow::anyhow!("key needs keys, such as \"enter\" or \"ctrl+s\""))?;
    let chord = input::parse_keys(&keys).map_err(|e| anyhow::anyhow!("{e}"))?;
    let w = drive(cx, "key", id, &[], None, json!({ "keys": keys }), move |_| input::win::press(&chord)).await?;
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
    let window = window_ref(&w, id)?;
    let app = app_label(&window.app);
    let body = DesktopReleaseRequest { auth: sign_ui_automation_auth()?, window };
    let answer = post(cx, "/api/v1/agent/desktop/release", &body, 30).await?;
    Ok(if answer["released"].as_bool() == Some(true) {
        format!("Gave back your access to {app}.")
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
        // The frame host is recognised whatever the path's case, so a Store
        // app is never keyed on it (window_ref).
        assert!(app_label(r"C:\WINDOWS\system32\ApplicationFrameHost.exe").eq_ignore_ascii_case(FRAME_HOST));
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

    #[test]
    fn a_drag_is_read_from_its_ends_and_waypoints() {
        let d = parse_drag(&json!({ "x": 1, "y": 2, "x2": 30, "y2": 40, "via": [{ "x": 10, "y": 10 }], "hold": "ctrl+shift" })).unwrap();
        assert_eq!(d.points, vec![(1, 2), (10, 10), (30, 40)]);
        assert_eq!(d.button, "left");
        assert_eq!(d.hold.len(), 2);
    }

    #[test]
    fn a_drag_that_cannot_be_one_is_refused_with_the_fix() {
        let err = |v: Value| parse_drag(&v).unwrap_err().to_string();
        assert!(err(json!({ "x": 1, "y": 2, "x2": 3 })).contains("y2 is missing"));
        assert!(err(json!({ "x": 5, "y": 5, "x2": 5, "y2": 5 })).contains("use click"));
        assert!(err(json!({ "x": 1, "y": 2, "x2": 3, "y2": 4, "button": "back" })).contains("unknown button"));
        assert!(err(json!({ "x": 1, "y": 2, "x2": 3, "y2": 4, "hold": "win" })).contains("not allowed"));
        assert!(err(json!({ "x": 1, "y": 2, "x2": 3, "y2": 4, "via": "nope" })).contains("list"));
        assert!(err(json!({ "x": 1, "y": 2, "x2": 3, "y2": 4, "via": [{ "x": 1 }] })).contains("x and y"));
        let many: Vec<Value> = (0..9).map(|i| json!({ "x": i, "y": i })).collect();
        assert!(err(json!({ "x": 1, "y": 2, "x2": 3, "y2": 4, "via": many })).contains("at most 8"));
    }

    #[test]
    fn a_drag_that_returns_to_its_start_by_way_of_a_waypoint_is_allowed() {
        let d = parse_drag(&json!({ "x": 5, "y": 5, "x2": 5, "y2": 5, "via": [{ "x": 50, "y": 5 }] })).unwrap();
        assert_eq!(d.points, vec![(5, 5), (50, 5), (5, 5)]);
    }

    #[test]
    fn a_region_is_read_from_the_call() {
        assert_eq!(parse_region(&json!({})).unwrap(), None);
        assert_eq!(parse_region(&json!({ "region": null })).unwrap(), None);
        assert_eq!(parse_region(&json!({ "region": { "x": 1, "y": 2, "w": 30, "h": 40 } })).unwrap(), Some((1, 2, 30, 40)));
        assert!(parse_region(&json!({ "region": { "x": 1, "y": 2, "w": 30 } })).unwrap_err().to_string().contains("h is missing"));
    }
}
