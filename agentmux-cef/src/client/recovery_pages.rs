// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Free functions for crash / low-memory recovery pages and the URL/budget
//! helpers they rely on. Extracted verbatim from client/mod.rs.

use std::collections::VecDeque;
use std::time::Instant;

use super::{CRASH_BUDGET_WINDOW, MEMORY_PAUSE_BUDGET, MEMORY_PAUSE_WINDOW};

/// Terminal "give up" page rendered when a browser exceeds `CRASH_BUDGET`
/// renderer crashes within `CRASH_BUDGET_WINDOW`. Unlike the normal recovery
/// page, this one has NO reload button and NO `frame.load_url` target — it
/// only offers Quit. That's the whole point: navigating away from it cannot
/// re-enter `on_render_process_terminated` and restart the loop.
///
/// Auto-closes after `CRASH_LOOP_AUTO_CLOSE_SECS` so the dead window doesn't
/// linger in the host's window registry indefinitely. When `window.close()`
/// fires, the existing `on_before_close` path runs and `ReportWindowClosed`
/// → launcher → `Event::WindowInstanceReleased` chain decrements the
/// user-visible window count (fix for #1117 follow-up "decouple window count
/// from window lifecycle"). A visible countdown gives the user time to read
/// the message; clicking "Close this window" (or any keystroke / mouse
/// activity) cancels the auto-close so it's never surprising.
pub(crate) fn crash_loop_terminal_page(reason: &str, error_code: i32, crashes_in_window: usize) -> String {
    use crate::client::helpers::html_escape;
    const CRASH_LOOP_AUTO_CLOSE_SECS: u32 = 30;
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>AgentMux — Crash loop</title>
<style>
:root {{ color-scheme: dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #1e1e2e; color: #cdd6f4;
       display: flex; justify-content: center; align-items: center;
       min-height: 100vh; margin: 0; padding: 24px; box-sizing: border-box; }}
.box {{ text-align: center; max-width: 560px; padding: 36px;
       background: #181825; border: 1px solid #313244; border-radius: 10px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.4); }}
.icon {{ font-size: 36px; line-height: 1; margin-bottom: 12px; }}
h1 {{ color: #f38ba8; font-size: 22px; margin: 0 0 6px 0; }}
.reason {{ color: #a6adc8; font-size: 14px; margin: 0 0 20px 0; font-style: italic; }}
p {{ color: #bac2de; line-height: 1.55; margin: 0 0 12px 0; font-size: 14px; }}
.countdown {{ color: #a6adc8; font-size: 13px; margin-top: 16px; }}
.countdown span {{ color: #f9e2af; font-weight: 600; }}
button {{ padding: 10px 22px; border: 1px solid #45475a; border-radius: 6px;
         background: #313244; color: #cdd6f4; cursor: pointer;
         font-size: 13px; font-family: inherit; margin-top: 16px; }}
button:hover {{ background: #45475a; border-color: #585b70; }}
.footer {{ color: #6c7086; font-size: 11px; margin-top: 18px;
          font-family: ui-monospace, monospace; }}
</style></head>
<body><div class="box" role="alertdialog">
<div class="icon">🛑</div>
<h1>Window stopped recovering</h1>
<p class="reason">Reason: {reason_safe}</p>
<p>This window crashed {crashes_in_window} times within {window_secs} seconds.
Auto-recovery is disabled to prevent a crash loop.</p>
<p>Your other AgentMux windows and your saved sessions are not affected —
they remain available. Close this window and open a fresh one to continue.</p>
<button onclick="window.close()">Close this window</button>
<p class="countdown">Auto-closing in <span id="countdown">{auto_close_secs}</span> s
(any keypress or click cancels)</p>
<div class="footer">error_code={error_code}</div>
</div>
<script>
// Auto-close so the dead window doesn't linger in the host window
// registry — when window.close() fires, on_before_close runs the
// normal ReportWindowClosed → WindowInstanceReleased chain and the
// UI window count snaps to the live count. Any user interaction
// cancels (so the message stays readable as long as the user wants).
let secs = {auto_close_secs};
const el = document.getElementById('countdown');
let cancelled = false;
const cancel = () => {{ cancelled = true; el.parentElement.style.display = 'none'; }};
document.addEventListener('keydown', cancel, {{ once: true }});
document.addEventListener('mousedown', cancel, {{ once: true }});
const tick = () => {{
    if (cancelled) return;
    secs -= 1;
    if (secs <= 0) {{ window.close(); return; }}
    el.textContent = String(secs);
    setTimeout(tick, 1000);
}};
setTimeout(tick, 1000);
</script>
</body></html>"#,
        reason_safe = html_escape(reason),
        window_secs = CRASH_BUDGET_WINDOW.as_secs(),
        crashes_in_window = crashes_in_window,
        error_code = error_code,
        auto_close_secs = CRASH_LOOP_AUTO_CLOSE_SECS,
    )
}

/// True when `url` is on the same origin as `base_url` (i.e. a live app URL we
/// can safely reuse for recovery). The boundary check after the prefix guards
/// against a port that is a numeric prefix of another (e.g. `:5173` matching
/// `:51730`): the char following the origin must be a path/query/fragment
/// separator or end-of-string. `base_url` is an origin with no path
/// (`http://127.0.0.1:<port>` or `http://localhost:<port>`).
pub(crate) fn url_on_origin(url: &str, base_url: &str) -> bool {
    url.strip_prefix(base_url).is_some_and(|rest| {
        rest.is_empty()
            || rest.starts_with('/')
            || rest.starts_with('?')
            || rest.starts_with('#')
    })
}

/// Build the navigation URL a crash-recovery / low-memory page sends the user
/// back to. Carries `ipc_port` + `ipc_token` and — critically — the window's
/// `windowLabel` when known, so a recovered secondary window doesn't
/// reinitialize as `main` (creation.rs adds the same `windowLabel` param on
/// first load; the frontend defaults a missing label to `main`). codex P2 #1229.
pub(crate) fn recovery_navigation_url(
    base_url: &str,
    ipc_port: u16,
    ipc_token: &str,
    window_label: Option<&str>,
) -> String {
    let sep = if base_url.contains('?') { "&" } else { "?" };
    match window_label {
        Some(lbl) => format!(
            "{base_url}{sep}ipc_port={ipc_port}&ipc_token={ipc_token}&windowLabel={lbl}"
        ),
        None => format!("{base_url}{sep}ipc_port={ipc_port}&ipc_token={ipc_token}"),
    }
}

/// Record a memory-pause for `now` into `hist`, pruning entries older than
/// `MEMORY_PAUSE_WINDOW` first, and return whether we are still within
/// `MEMORY_PAUSE_BUDGET`. Extracted from `on_render_process_terminated` so the
/// bounded-recovery logic — the part that must converge on the give-up page
/// under *total* commit exhaustion rather than loop forever — is unit-testable
/// without a live CEF browser. (SPEC_GATED_RENDERER_RECOVERY §6.B.)
pub(crate) fn record_memory_pause(hist: &mut VecDeque<Instant>, now: Instant) -> bool {
    while hist.front().is_some_and(|t| now.duration_since(*t) > MEMORY_PAUSE_WINDOW) {
        hist.pop_front();
    }
    hist.push_back(now);
    hist.len() <= MEMORY_PAUSE_BUDGET
}

/// Low-memory "paused" page — shown when a renderer is OOM-terminated while the
/// system commit limit is exhausted (SPEC_GATED_RENDERER_RECOVERY §6.B). Unlike
/// the give-up page this state is RECOVERABLE: all durable state lives in the
/// sidecar, so "Resume" navigates to the live app URL and re-projects
/// everything — losing nothing. The Resume is manual and memory-guided: an
/// automatic retry before commit recovers would just re-OOM, so we tell the
/// user to free memory first (host-driven, memory-gated auto-resume is Phase
/// 1b). `app_url` is the live frontend URL Resume navigates to (spawns a fresh
/// renderer); it already carries the ipc_token, same as the recovery page.
pub(crate) fn memory_paused_page(reason: &str, error_code: i32, commit_free_mb: u64, app_url: &str) -> String {
    use crate::client::helpers::{html_escape, js_string_literal};
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>AgentMux — Low memory</title>
<style>
:root {{ color-scheme: dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #1e1e2e; color: #cdd6f4;
       display: flex; justify-content: center; align-items: center;
       min-height: 100vh; margin: 0; padding: 24px; box-sizing: border-box; }}
.box {{ text-align: center; max-width: 560px; padding: 36px;
       background: #181825; border: 1px solid #313244; border-radius: 10px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.4); }}
.icon {{ font-size: 36px; line-height: 1; margin-bottom: 12px; }}
h1 {{ color: #f9e2af; font-size: 22px; margin: 0 0 6px 0; }}
.reason {{ color: #a6adc8; font-size: 14px; margin: 0 0 20px 0; font-style: italic; }}
p {{ color: #bac2de; line-height: 1.55; margin: 0 0 12px 0; font-size: 14px; }}
strong {{ color: #f9e2af; }}
.actions {{ display: flex; gap: 10px; justify-content: center; margin-top: 24px; flex-wrap: wrap; }}
button {{ padding: 10px 22px; border: 1px solid #45475a; border-radius: 6px;
         background: #313244; color: #cdd6f4; cursor: pointer;
         font-size: 13px; font-family: inherit; }}
button:hover {{ background: #45475a; border-color: #585b70; }}
button.primary {{ background: #89b4fa; color: #1e1e2e; border-color: #89b4fa; font-weight: 600; }}
button.primary:hover {{ background: #74a0f8; border-color: #74a0f8; }}
.footer {{ color: #6c7086; font-size: 11px; margin-top: 18px; font-family: ui-monospace, monospace; }}
</style></head>
<body><div class="box" role="alertdialog">
<div class="icon">⏳</div>
<h1>Paused — system memory low</h1>
<p class="reason">Reason: {reason_safe}</p>
<p>This window paused because the system ran out of memory
(only {commit_free_mb} MB of commit was free). <strong>Your work is safe</strong> —
everything is saved in the background and will be restored exactly when this
window resumes.</p>
<p><strong>Free some memory first</strong> — close other AgentMux windows or
other apps — then click Resume. Resuming before memory recovers will just
pause again.</p>
<div class="actions">
<button id="amx-resume" class="primary">Resume</button>
<button id="amx-quit">Quit this window</button>
</div>
<div class="footer">error_code={error_code} · commit_free={commit_free_mb}MB</div>
</div>
<script>
(function(){{
  var r = document.getElementById('amx-resume');
  if (r) r.addEventListener('click', function () {{ location.href = {app_url_js}; }});
  var q = document.getElementById('amx-quit');
  if (q) q.addEventListener('click', function () {{ window.close(); }});
}})();
</script>
</body></html>"#,
        reason_safe = html_escape(reason),
        commit_free_mb = commit_free_mb,
        error_code = error_code,
        app_url_js = js_string_literal(app_url),
    )
}

#[cfg(test)]
mod gated_recovery_tests {
    use super::{memory_paused_page, record_memory_pause, url_on_origin};
    use crate::client::{MEMORY_PAUSE_BUDGET, MEMORY_PAUSE_WINDOW, RESUME_FLOOR_MB};
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    #[test]
    fn memory_paused_page_resume_button_has_working_handler() {
        // Regression for the inert-Resume bug: js_string_literal() returns a
        // DOUBLE-quoted JS string and is a <script>-context literal; embedding it
        // in a double-quoted onclick="..." attribute terminated the attribute
        // early, so Resume did nothing and the window wedged. The handler must be
        // wired in a <script> block instead.
        let url = "http://127.0.0.1:63627/?ipc_port=63627&ipc_token=abc";
        let html = memory_paused_page("out of memory", -1, 100, url);

        // The broken antipattern must be gone.
        assert!(
            !html.contains("onclick=\"location.href"),
            "Resume must not use an inline onclick with a double-quoted URL"
        );
        // The handler is attached by id via addEventListener.
        assert!(html.contains("addEventListener"), "handler should use addEventListener");
        assert!(html.contains("id=\"amx-resume\""), "Resume button needs its id");
        // The app URL must appear as a valid JS string literal in script context
        // (js_string_literal escapes & -> \\u0026 and wraps in double quotes).
        assert!(
            html.contains(
                "location.href = \"http://127.0.0.1:63627/?ipc_port=63627\\u0026ipc_token=abc\""
            ),
            "Resume must navigate to the app URL as a valid JS string literal"
        );
    }

    #[test]
    fn url_on_origin_matches_only_real_same_origin_urls() {
        let base = "http://localhost:5173";
        // Exact origin, origin + path / query / fragment — all reuse.
        assert!(url_on_origin("http://localhost:5173", base));
        assert!(url_on_origin("http://localhost:5173/", base));
        assert!(url_on_origin("http://localhost:5173/?windowLabel=w1&workspaceId=ws", base));
        assert!(url_on_origin("http://localhost:5173?ipc_port=1&ipc_token=t", base));
        assert!(url_on_origin("http://localhost:5173#/route", base));
        // Port that merely *extends* the base port must NOT match.
        assert!(!url_on_origin("http://localhost:51730/?x=1", base));
        // Different origin, and a non-http (data:) recovery page, must not match.
        assert!(!url_on_origin("http://127.0.0.1:5173/?x=1", base));
        assert!(!url_on_origin("data:text/html;base64,abc", base));
        // Prod origin behaves the same.
        let prod = "http://127.0.0.1:8080";
        assert!(url_on_origin("http://127.0.0.1:8080/?ipc_port=8080", prod));
        assert!(!url_on_origin("http://127.0.0.1:80801/", prod));
    }

    #[test]
    fn resume_floor_is_above_a_fresh_renderer_commit() {
        // A fresh renderer commits ~100-200 MB; the floor must leave margin so
        // a resume doesn't instantly re-OOM. Guards against an accidental
        // shrink that would make the gate useless.
        assert!(RESUME_FLOOR_MB >= 256, "RESUME_FLOOR_MB too low to be safe");
    }

    #[test]
    fn within_budget_until_exceeded_then_converges() {
        let mut hist: VecDeque<Instant> = VecDeque::new();
        let now = Instant::now();
        // The first MEMORY_PAUSE_BUDGET pauses stay within budget (pause path).
        for i in 0..MEMORY_PAUSE_BUDGET {
            assert!(
                record_memory_pause(&mut hist, now),
                "pause {} should be within budget",
                i + 1
            );
        }
        // The next one exceeds → falls through to the give-up path. This is the
        // total-exhaustion convergence guarantee (no infinite memory-pause loop).
        assert!(
            !record_memory_pause(&mut hist, now),
            "exceeding the budget must return false (fall through to give-up)"
        );
    }

    #[test]
    fn old_pauses_outside_the_window_are_pruned() {
        let mut hist: VecDeque<Instant> = VecDeque::new();
        let start = Instant::now();
        // Fill the budget at t=start.
        for _ in 0..MEMORY_PAUSE_BUDGET {
            record_memory_pause(&mut hist, start);
        }
        // Later than the window: all prior entries prune, so we're within budget
        // again — a window that recovered then hit pressure again is NOT treated
        // as a wedged loop.
        let later = start + MEMORY_PAUSE_WINDOW + Duration::from_secs(1);
        assert!(
            record_memory_pause(&mut hist, later),
            "pauses older than the window must be pruned, resetting the budget"
        );
        assert_eq!(hist.len(), 1, "only the recent pause should remain");
    }
}
