// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolve `block_id` → CDP target id.
//!
//! CEF's `/json` endpoint exposes every active page target but does
//! NOT expose the underlying `cef::Browser::identifier()`, so we can't
//! match by CEF id directly. Two resolution paths:
//!
//! **Path 1 — dedicated browser-pane block** (a `view: "browser"` widget,
//! embedding its own separate CEF `Browser` for arbitrary third-party
//! content): resolve by URL match, as originally shipped.
//! 1. Ask the pane manager for the pane's current URL.
//! 2. Probe `GET http://127.0.0.1:<debug>/json`.
//! 3. Find the entry whose `url` matches the pane's URL AND whose
//!    `id` isn't already owned by some other cached block.
//!
//! **Path 2 — any other block** (agent/terminal/editor/etc. panes, which
//! are DOM nodes inside a SHARED main/pool/floating window's page, not
//! their own CDP target — see `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`
//! §1): there is no URL-based way to disambiguate which window a given
//! block lives in, so instead probe every "page" target directly and ask
//! each one (via one `Runtime.evaluate`) whether its DOM contains a
//! `[data-blockid="<id>"]` element (set on every pane's root wrapper by
//! `frontend/app/block/blockframe.tsx`). First match wins.
//!
//! Callers MUST know which path resolved a target, because it changes
//! whether DOM-subtree scoping is required: a Path-2 target is a SHARED
//! page (other panes' DOM lives there too — queries/clicks/screenshots
//! must scope to `[data-blockid]`), while a Path-1 target already IS the
//! block's own isolated page (arbitrary third-party content with no
//! `data-blockid` concept — scoping would break it). See `ResolvedTarget`.
//!
//! Cache: `(block_id → ResolvedTarget)`. Two browser panes on the same URL
//! used to be indistinguishable here, so a block could be handed ANOTHER
//! pane's page: screenshots came back sized for the wrong pane, and `eval`/
//! `navigate` could act on the wrong one. When more than one unclaimed
//! target matches, Path 1 now asks the block's own `Browser` to stamp its
//! page with a one-off value and picks the target whose page carries it
//! (`pick_by_stamp`).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Deserialize;

use super::cdp::CdpSession;
use crate::state::AppState;

pub type ResolveError = String;

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub target_id: String,
    /// True when this target is a page SHARED with other panes (the main
    /// AgentMux UI, or a pool/floating window) — callers must scope
    /// queries/clicks/screenshots to this block's own `[data-blockid]`
    /// subtree. False for a dedicated browser-pane target, which already
    /// IS this block's own page.
    pub scope_to_block: bool,
}

#[derive(Default)]
pub struct TargetCache {
    // block_id → resolved target
    entries: Mutex<HashMap<String, ResolvedTarget>>,
}

#[derive(Debug, Deserialize)]
struct JsonTarget {
    id: String,
    url: String,
    #[serde(default, rename = "type")]
    kind: String,
}

impl TargetCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn resolve(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
    ) -> Result<ResolvedTarget, ResolveError> {
        // Fast path: cache hit.
        if let Some(cached) = self.entries.lock().get(block_id).cloned() {
            return Ok(cached);
        }

        let debug_port = *state.debug_port.lock();
        if debug_port == 0 {
            return Err("CEF debug port not yet configured".to_string());
        }

        // Path 1: a dedicated browser-pane block — resolve by URL match,
        // exactly as originally shipped. Target exclusivity (via
        // already_cached) matters here: two browser panes must never be
        // handed the same dedicated target.
        let resolved = if let Some(pane_url) = state.browser_panes.pane_url(state, block_id) {
            let already_cached = self.already_cached();
            let candidates = Self::url_candidates(debug_port, &pane_url, &already_cached).await?;
            let target_id = match candidates.as_slice() {
                [] => {
                    return Err(format!(
                        "UNKNOWN_BLOCK_ID: no unclaimed CDP target matches url={pane_url} \
                         for block_id={block_id}"
                    ))
                }
                [only] => only.clone(),
                _ => {
                    let stamp = uuid::Uuid::new_v4().simple().to_string();
                    if !state
                        .browser_panes
                        .run_js_in_pane(state, block_id, &stamp_js(&stamp))
                    {
                        return Err(format!("block_id={block_id}: pane closed while resolving"));
                    }
                    Self::pick_by_stamp(debug_port, &candidates, &stamp, STAMP_POLLS, STAMP_POLL_INTERVAL)
                        .await
                        .ok_or_else(|| {
                            format!(
                                "UNKNOWN_BLOCK_ID: {} CDP targets match url={pane_url} and none \
                                 carried block_id={block_id}'s stamp",
                                candidates.len()
                            )
                        })?
                }
            };
            ResolvedTarget {
                target_id,
                scope_to_block: false,
            }
        } else {
            // Path 2 (fallback): not a browser pane — a DOM node inside
            // one of the shared window pages. Probe. NOT target-exclusive
            // — see resolve_by_dom_probe's doc comment for why.
            let target_id = Self::resolve_by_dom_probe(debug_port, block_id).await?;
            ResolvedTarget {
                target_id,
                scope_to_block: true,
            }
        };

        self.entries
            .lock()
            .insert(block_id.to_string(), resolved.clone());
        Ok(resolved)
    }

    fn already_cached(&self) -> Vec<String> {
        self.entries
            .lock()
            .values()
            .map(|r| r.target_id.clone())
            .collect()
    }

    async fn fetch_page_targets(debug_port: u16) -> Result<Vec<JsonTarget>, ResolveError> {
        let json_url = format!("http://127.0.0.1:{debug_port}/json");
        let resp = reqwest::get(&json_url)
            .await
            .map_err(|e| format!("GET {json_url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{json_url} returned {}", resp.status()));
        }
        resp.json()
            .await
            .map_err(|e| format!("parse /json: {e}"))
    }

    /// Every unclaimed page target whose URL matches the pane's, in `/json`
    /// order. More than one means several panes are on the same URL, which
    /// only `pick_by_stamp` can tell apart.
    async fn url_candidates(
        debug_port: u16,
        pane_url: &str,
        already_cached: &[String],
    ) -> Result<Vec<String>, ResolveError> {
        let targets = Self::fetch_page_targets(debug_port).await?;

        // Filter:
        // - type=="page" (skip worker, iframe, etc.)
        // - url matches the pane's url (exact or trailing-slash-tolerant)
        // - id not already in our cache (avoids claiming another block's target)
        let pane_url_norm = normalize_url(pane_url);
        Ok(targets
            .iter()
            .filter(|t| t.kind == "page" || t.kind.is_empty())
            .filter(|t| !already_cached.contains(&t.id))
            .filter(|t| normalize_url(&t.url) == pane_url_norm)
            .map(|t| t.id.clone())
            .collect())
    }

    /// Which of `candidates` is the page stamped with `stamp` (`stamp_js`,
    /// run in the block's own `Browser`). Polled, because the stamp is
    /// posted to the renderer and lands a little later; the matching page's
    /// stamp is removed once found. None if no page shows it in time.
    async fn pick_by_stamp(
        debug_port: u16,
        candidates: &[String],
        stamp: &str,
        polls: u32,
        interval: std::time::Duration,
    ) -> Option<String> {
        let mut sessions = Vec::new();
        for id in candidates {
            let ws_url = format!("ws://127.0.0.1:{debug_port}/devtools/page/{id}");
            if let Ok(cdp) = CdpSession::connect(&ws_url).await {
                sessions.push((id.clone(), cdp));
            }
        }
        let probe = stamp_probe_js(stamp);
        let mut found = None;
        'poll: for attempt in 0..polls {
            if attempt > 0 {
                tokio::time::sleep(interval).await;
            }
            for (id, cdp) in sessions.iter_mut() {
                let reply = cdp
                    .call(
                        "Runtime.evaluate",
                        serde_json::json!({ "expression": probe, "returnByValue": true }),
                    )
                    .await;
                let hit = reply
                    .ok()
                    .and_then(|v| v.get("result").and_then(|r| r.get("value")).cloned())
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if hit {
                    found = Some(id.clone());
                    break 'poll;
                }
            }
        }
        for (_, cdp) in sessions {
            cdp.close().await;
        }
        found
    }

    /// Probe every "page" CDP target and ask each one (via a cheap
    /// `Runtime.evaluate`) whether its DOM contains this block's
    /// `[data-blockid]` wrapper. There are normally only a handful of live
    /// top-level windows, so this is fast; no window-topology bookkeeping
    /// (host↔srv round trip) is needed.
    ///
    /// Deliberately does NOT exclude already-cached/claimed targets the way
    /// `url_candidates` does — that exclusivity is a Path-1 (dedicated
    /// browser-pane) concept, where one CDP target really can only belong
    /// to one block. Path 2's whole premise is the opposite: MANY blocks
    /// (every pane in a window) legitimately share ONE target. Applying
    /// Path-1's exclusivity here was a real bug (caught during live
    /// verification, 2026-08-19): the first block resolved in a window
    /// would "claim" the only target, and every other block in that SAME
    /// window would then fail to resolve at all — see
    /// `resolve_by_dom_probe_lets_multiple_blocks_share_one_window_target`.
    async fn resolve_by_dom_probe(
        debug_port: u16,
        block_id: &str,
    ) -> Result<String, ResolveError> {
        let targets = Self::fetch_page_targets(debug_port).await?;
        // JSON-encode block_id so it's a safe JS string literal; the
        // selector itself is built via CSS.escape inside the expression,
        // not string-concatenated here, so a block_id containing quotes
        // can't break out of the selector.
        let block_id_js = serde_json::to_string(block_id).unwrap_or_else(|_| "\"\"".to_string());
        let probe_expr = format!(
            "(() => {{ try {{ return !!document.querySelector(\
             '[data-blockid=\"' + CSS.escape({block_id_js}) + '\"]'); \
             }} catch (e) {{ return false; }} }})()"
        );

        for t in targets
            .iter()
            .filter(|t| t.kind == "page" || t.kind.is_empty())
        {
            let ws_url = format!("ws://127.0.0.1:{debug_port}/devtools/page/{}", t.id);
            let mut cdp = match CdpSession::connect(&ws_url).await {
                Ok(c) => c,
                Err(_) => continue, // target may have closed between /json and connect
            };
            let reply = cdp
                .call(
                    "Runtime.evaluate",
                    serde_json::json!({ "expression": probe_expr, "returnByValue": true }),
                )
                .await;
            let _ = cdp.close().await;
            let found = reply
                .ok()
                .and_then(|v| v.get("result").and_then(|r| r.get("value")).cloned())
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if found {
                return Ok(t.id.clone());
            }
        }

        let probed = targets
            .iter()
            .filter(|t| t.kind == "page" || t.kind.is_empty())
            .count();
        Err(format!(
            "UNKNOWN_BLOCK_ID: no CDP target's DOM contains a [data-blockid=\"{block_id}\"] \
             element (probed {probed} page targets)"
        ))
    }

    /// Invalidate a block's cached target — called when a pane closes
    /// or navigates. On next resolve we'll re-probe.
    pub fn forget(&self, block_id: &str) {
        self.entries.lock().remove(block_id);
    }
}

/// Window property the stamp lives in while a same-URL resolve is in flight.
const STAMP_KEY: &str = "__agentmuxResolveStamp";
/// ~600ms in all: the stamp normally lands within a frame or two.
const STAMP_POLLS: u32 = 20;
const STAMP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(30);

/// JS that stamps the page it runs in. Non-enumerable, so the page's own
/// scripts don't trip over it walking `window`; removed by `stamp_probe_js`.
fn stamp_js(stamp: &str) -> String {
    format!(
        "Object.defineProperty(window, '{STAMP_KEY}', {{ value: '{stamp}', configurable: true }});"
    )
}

/// JS that answers whether this page carries `stamp`, removing it if so.
fn stamp_probe_js(stamp: &str) -> String {
    format!(
        "(() => {{ const hit = window['{STAMP_KEY}'] === '{stamp}'; \
         if (hit) delete window['{STAMP_KEY}']; return hit; }})()"
    )
}

fn normalize_url(u: &str) -> String {
    // `https://www.google.com` vs `https://www.google.com/` are the
    // same target; CEF /json includes the trailing slash, the pane's
    // meta.url in our tests usually doesn't.
    let trimmed = u.trim_end_matches('/').to_string();
    trimmed.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{normalize_url, stamp_js, stamp_probe_js, TargetCache};

    #[test]
    fn normalize_strips_trailing_slash_and_lowercases() {
        assert_eq!(
            normalize_url("https://www.google.com/"),
            normalize_url("https://WWW.GOOGLE.COM"),
        );
    }

    /// Minimal fake CDP `/json` endpoint — serves `body` verbatim for every
    /// GET. Enough to exercise url_candidates / resolve_by_dom_probe's
    /// target-matching logic without a real CEF process.
    async fn spawn_fake_json_endpoint(body: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 2048];
                    let Ok(_) = stream.read(&mut buf).await else { return };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        port
    }

    // Path 1 (dedicated browser-pane block): matches by normalized URL, and
    // — critically for the multi-pane case — must not re-claim a target
    // another cached block already owns even when the URL still matches.
    #[tokio::test]
    async fn url_candidates_match_and_skip_already_claimed_targets() {
        let port = spawn_fake_json_endpoint(
            r#"[
                {"id": "target-a", "type": "page", "url": "https://example.com/pane-a/"},
                {"id": "target-b", "type": "page", "url": "https://example.com/pane-b"}
            ]"#,
        )
        .await;

        let found = TargetCache::url_candidates(port, "https://example.com/pane-a", &[])
            .await
            .expect("/json is reachable");
        assert_eq!(found, vec!["target-a".to_string()]);

        let claimed = TargetCache::url_candidates(
            port,
            "https://example.com/pane-a",
            &["target-a".to_string()],
        )
        .await
        .expect("/json is reachable");
        assert!(claimed.is_empty(), "target-a is already claimed, must not be handed out twice");
    }

    // Several panes on one URL: every one of them is a candidate, which is
    // what sends resolve() to pick_by_stamp instead of taking the first.
    #[tokio::test]
    async fn url_candidates_returns_every_same_url_target() {
        let port = spawn_fake_json_endpoint(
            r#"[
                {"id": "target-a", "type": "page", "url": "https://agentmux.ai/"},
                {"id": "target-b", "type": "page", "url": "https://agentmux.ai/"},
                {"id": "target-c", "type": "page", "url": "https://example.com/"}
            ]"#,
        )
        .await;

        let found = TargetCache::url_candidates(port, "https://agentmux.ai", &[])
            .await
            .expect("/json is reachable");
        assert_eq!(found, vec!["target-a".to_string(), "target-b".to_string()]);
    }

    // The stamped page is found even when it isn't the first candidate —
    // the old first-URL-match rule handed block B target-a here.
    #[tokio::test]
    async fn pick_by_stamp_finds_the_stamped_page_among_same_url_targets() {
        let port = spawn_fake_cdp_endpoint(r#"[]"#, Some("target-b")).await;
        let found = TargetCache::pick_by_stamp(
            port,
            &["target-a".to_string(), "target-b".to_string()],
            "stamp",
            3,
            std::time::Duration::from_millis(1),
        )
        .await;
        assert_eq!(found.as_deref(), Some("target-b"));
    }

    #[tokio::test]
    async fn pick_by_stamp_gives_up_when_no_page_carries_the_stamp() {
        let port = spawn_fake_cdp_endpoint(r#"[]"#, Some("target-z")).await;
        let found = TargetCache::pick_by_stamp(
            port,
            &["target-a".to_string(), "target-b".to_string()],
            "stamp",
            2,
            std::time::Duration::from_millis(1),
        )
        .await;
        assert_eq!(found, None);
    }

    #[test]
    fn stamp_js_and_probe_agree_on_the_key_and_value() {
        let set = stamp_js("abc123");
        let probe = stamp_probe_js("abc123");
        assert!(set.contains("'__agentmuxResolveStamp'") && set.contains("'abc123'"), "{set}");
        assert!(probe.contains("'__agentmuxResolveStamp'") && probe.contains("'abc123'"), "{probe}");
        assert!(probe.contains("delete window"), "the probe removes the stamp once found: {probe}");
    }

    // Path 2 (fallback — any other block): no real CDP server sits behind
    // these fake targets, so every connect attempt fails and this exercises
    // the "probed N targets, none matched" error path without needing to
    // fake the CDP WebSocket protocol itself.
    #[tokio::test]
    async fn resolve_by_dom_probe_reports_a_clear_error_when_nothing_matches() {
        let port = spawn_fake_json_endpoint(
            r#"[{"id": "target-a", "type": "page", "url": "https://example.com/"}]"#,
        )
        .await;

        let err = TargetCache::resolve_by_dom_probe(port, "some-block-id")
            .await
            .expect_err("no CDP target is actually reachable in this test");
        assert!(err.contains("UNKNOWN_BLOCK_ID"), "got: {err}");
        assert!(err.contains("some-block-id"), "got: {err}");
    }

    /// Minimal fake CDP endpoint: serves `/json` (like `spawn_fake_json_endpoint`)
    /// AND accepts a WebSocket at `/devtools/page/<id>`, answering every
    /// `Runtime.evaluate` call with `{result: {value: true}}` — i.e. "yes,
    /// this target's DOM contains whatever [data-blockid] selector was
    /// asked about." Good enough to exercise the DOM-probe *success* path,
    /// which the connect-always-fails tests above can't reach.
    /// `true_only_on`: answer true only on that target's socket, false on
    /// every other — "only this page carries the stamp."
    async fn spawn_fake_cdp_endpoint(json_body: &'static str, true_only_on: Option<&'static str>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    // Peek (not consume) the request line to route: `/json`
                    // is a plain HTTP GET, `/devtools/page/<id>` is a WS
                    // upgrade. Using `peek` (rather than reading via a
                    // BufReader and calling `.into_inner()`) matters: `
                    // BufReader::into_inner()` DISCARDS whatever it already
                    // buffered past the first line, desyncing the stream
                    // and hanging `accept_async`'s handshake read forever —
                    // that was a real bug in an earlier version of this
                    // test helper (2026-08-19), not in production code.
                    let mut peek_buf = [0u8; 128];
                    let Ok(n) = stream.peek(&mut peek_buf).await else { return };
                    // The request line's path, e.g. `/devtools/page/target-b`.
                    let path = std::str::from_utf8(&peek_buf[..n])
                        .ok()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("")
                        .to_string();
                    let answer = match true_only_on {
                        Some(id) => path.ends_with(&format!("/{id}")),
                        None => true,
                    };
                    if peek_buf[..n].starts_with(b"GET /json") {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        let mut stream = stream;
                        let mut buf = [0u8; 1024];
                        let _ = stream.read(&mut buf).await; // drain the request, best-effort
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            json_body.len(),
                            json_body
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                        return;
                    }
                    // WebSocket upgrade for /devtools/page/<id>.
                    let mut ws = match tokio_tungstenite::accept_async(stream).await {
                        Ok(w) => w,
                        Err(_) => return,
                    };
                    use futures_util::{SinkExt, StreamExt};
                    while let Some(Ok(msg)) = ws.next().await {
                        let tokio_tungstenite::tungstenite::Message::Text(text) = msg else {
                            continue;
                        };
                        let Ok(req): Result<serde_json::Value, _> = serde_json::from_str(&text)
                        else {
                            continue;
                        };
                        let id = req.get("id").cloned().unwrap_or(serde_json::json!(0));
                        let reply = serde_json::json!({ "id": id, "result": { "result": { "value": answer } } });
                        if ws
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                reply.to_string().into(),
                            ))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        port
    }

    // Regression test for a real bug caught during live verification
    // (2026-08-19): two DIFFERENT blocks that legitimately live in the SAME
    // shared window (e.g. an "agent" pane and a "sysinfo" pane both on the
    // main window) must BOTH resolve successfully to that one shared
    // target. The `already_cached` target-exclusivity check that Path 1
    // needs (two browser panes must never be handed the same dedicated
    // target) was wrongly also applied to Path 2, where "many blocks, one
    // shared target" is the whole point — the first block_id resolved
    // would "claim" the only target, so any lookup for a different block
    // in the SAME window then failed with UNKNOWN_BLOCK_ID even though its
    // [data-blockid] element genuinely exists on that page.
    #[tokio::test]
    async fn resolve_by_dom_probe_lets_multiple_blocks_share_one_window_target() {
        let port = spawn_fake_cdp_endpoint(
            r#"[{"id": "main-window-target", "type": "page", "url": "http://127.0.0.1:5307/"}]"#,
            None,
        )
        .await;

        let first = TargetCache::resolve_by_dom_probe(port, "agent-block-id")
            .await
            .expect("first block in the shared window should resolve");
        assert_eq!(first, "main-window-target");

        // The critical assertion: a SECOND, different block_id in the SAME
        // window must ALSO resolve — this is what broke before the fix,
        // when resolve() fed the first block's already-cached target_id
        // into an exclusivity filter this function no longer has.
        let second = TargetCache::resolve_by_dom_probe(port, "sysinfo-block-id")
            .await
            .expect(
                "a second block sharing the same window target must still resolve — \
                 target exclusivity is a Path-1 (dedicated browser pane) concept, \
                 not a Path-2 (shared window) one",
            );
        assert_eq!(second, "main-window-target");
    }
}
