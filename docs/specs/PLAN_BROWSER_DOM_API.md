# PLAN: Browser-pane DOM API implementation

Companion to `docs/specs/SPEC_BROWSER_DOM_API.md`. The spec says
**what** to build; this plan says **how**, in the order an
implementor should actually tackle it.

Each phase maps to one PR. Every phase is independently testable and
leaves the repo in a working state.

---

## Phase 0 — prerequisites (done in spec PR)

- ✅ Spec committed at `docs/specs/SPEC_BROWSER_DOM_API.md`.
- ✅ Spec auth header corrected to `Authorization: Bearer <ipc_token>`
  (matches `agentmux-cef/src/ipc.rs:131-136`).
- ✅ Confirmed dep chain already carries `tokio-tungstenite` + `reqwest`
  via `agentmux-srv`'s tree (so no new top-level deps needed — just
  add them to `agentmux-cef/Cargo.toml`).
- ✅ Confirmed `remote_debugging_port` = 9223 dev / 9222 release in
  `agentmux-cef/src/main.rs:325`.
- ✅ Confirmed CEF exposes `/json` target list on that port.

## Phase 1 — CDP client + target resolver + first read endpoint

**PR shape**: minimum-viable DOM API. One read method,
`browser.query`, going through a brand-new plumbing stack. All later
methods layer onto this same infrastructure.

**Validation gate**: `dom-smoke.ps1` can open a 3-pane layout, call
`browser.query` for `input[name='q']` in P1, assert the response
carries a non-empty `matches` array with a plausible `rect`.

### Files to add

```
agentmux-cef/src/browser_api/
├── mod.rs           (~60 lines — module root, route registrar)
├── cdp.rs           (~180 lines — WebSocket client + message pump)
├── resolver.rs      (~120 lines — block_id → CDP target id)
├── routes.rs        (~150 lines — axum handlers for this phase)
├── types.rs         (~80 lines — request/response structs)
└── scripts/
    └── query.js    (~40 lines — DOM query helper)
```

### Files to change

| File | Change |
|---|---|
| `agentmux-cef/src/main.rs` | `mod browser_api;` |
| `agentmux-cef/src/ipc.rs` | Extract auth middleware into a reusable extractor (`BearerAuth`), then register `/agentmux/browser/*` routes via `browser_api::register_routes(router, state)`. |
| `agentmux-cef/src/browser_panes.rs` | Add `browser_identifier_for(block_id: &str) -> Option<i32>` returning the CEF `Browser.identifier()` for the pane. This is all `resolver.rs` needs from the pane manager. |
| `agentmux-cef/Cargo.toml` | Add `tokio-tungstenite = "0.24"` (or whichever the workspace uses — align with `agentmux-srv`). `futures-util` for WS stream helpers. |

### Key types

```rust
// agentmux-cef/src/browser_api/types.rs
#[derive(serde::Deserialize)]
pub struct QueryReq {
    pub block_id: String,
    pub selector: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(serde::Serialize)]
pub struct Element {
    pub selector: String,   // unique path the backend computed
    pub tag: String,
    pub text: String,
    pub attrs: serde_json::Map<String, serde_json::Value>,
    pub rect: Rect,
    pub focused: bool,
}

#[derive(serde::Serialize)]
pub struct Rect { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiResponse<T> {
    Ok { data: T },
    Err { error: String },
}
```

### CDP client sketch (cdp.rs)

```rust
pub struct CdpSession {
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    stream: SplitStream<...>,
    next_id: std::sync::atomic::AtomicU64,
}

impl CdpSession {
    pub async fn connect(debugger_url: &str) -> Result<Self, CdpError>;
    pub async fn call(&mut self, method: &str, params: Value)
        -> Result<Value, CdpError>;   // blocks until reply with matching id
    pub async fn close(self);
}
```

One WS per API request in this phase. Fine for localhost (~10 ms per
round-trip). Pooling is Phase 5 — only implement it if measurement
shows it matters.

### Resolver sketch (resolver.rs)

```rust
pub struct TargetCache {
    // (block_id, cef_browser_id) → cdp_target_id
    entries: parking_lot::Mutex<HashMap<(String, i32), String>>,
}

impl TargetCache {
    pub async fn resolve(
        &self,
        pane_mgr: &BrowserPaneManager,
        debug_port: u16,
        block_id: &str,
    ) -> Result<String, ResolveError>;
}
```

`resolve()`:
1. `let id = pane_mgr.browser_identifier_for(block_id).ok_or(UnknownBlock)?;`
2. Check cache for `(block_id, id)`. Hit → return.
3. Miss: `GET http://127.0.0.1:{debug_port}/json` via `reqwest`,
   parse array of targets, find the one whose `browserId`
   (or equivalent — verify during implementation) matches `id`.
4. Insert into cache, return target id.

Invalidation: on pane close, call `TargetCache::drop_block(block_id)`.
Wire into `BrowserPaneManager::close` after it empties `state.browsers`.

### Route handler sketch (routes.rs)

```rust
pub async fn query(
    BearerAuth(_token): BearerAuth,
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryReq>,
) -> Json<ApiResponse<QueryData>> {
    let target = match state.browser_api
        .target_cache
        .resolve(&state.browser_panes, state.debug_port, &req.block_id)
        .await
    {
        Ok(t) => t,
        Err(e) => return Json(ApiResponse::err(e.to_string())),
    };

    let ws_url = format!(
        "ws://127.0.0.1:{}/devtools/page/{}",
        state.debug_port, target
    );

    let mut cdp = match CdpSession::connect(&ws_url).await {
        Ok(s) => s, Err(e) => return err_resp(e),
    };

    // Inject query helper (idempotent — our script guards with
    // `if (!window.__amq_query) { ... }`).
    let helper = include_str!("scripts/query.js");
    let _ = cdp.call("Runtime.evaluate", json!({
        "expression": helper,
        "returnByValue": false,
    })).await;

    let call_expr = format!(
        "__amq_query({}, {})",
        serde_json::to_string(&req.selector).unwrap(),
        req.limit.unwrap_or(0),
    );
    let result = match cdp.call("Runtime.evaluate", json!({
        "expression": call_expr,
        "returnByValue": true,
    })).await {
        Ok(v) => v,
        Err(e) => return err_resp(e),
    };

    let _ = cdp.close().await;

    let matches = serde_json::from_value(result["result"]["value"].clone())
        .unwrap_or_default();
    Json(ApiResponse::ok(QueryData { matches }))
}
```

### `query.js` sketch

```js
(() => {
  if (window.__amq_query) return;
  window.__amq_query = (selector, limit) => {
    const nodes = document.querySelectorAll(selector);
    const out = [];
    const n = limit > 0 ? Math.min(limit, nodes.length) : nodes.length;
    for (let i = 0; i < n; i++) {
      const el = nodes[i];
      const r = el.getBoundingClientRect();
      const attrs = {};
      for (const a of el.attributes) attrs[a.name] = a.value;
      out.push({
        selector: __amq_path(el),     // derived unique path
        tag: el.tagName.toLowerCase(),
        text: (el.textContent || '').slice(0, 500),
        attrs,
        rect: { x: r.x, y: r.y, width: r.width, height: r.height },
        focused: el === document.activeElement,
      });
    }
    return out;
  };
  window.__amq_path = /* nth-of-type chain up to body */;
})();
```

### Tests

1. **Unit** (`agentmux-cef/tests/cdp_roundtrip.rs`): start a minimal
   in-process CDP-mimicking WS server, connect with `CdpSession`,
   send a `Runtime.evaluate`-like call, verify reply threading
   (response `id` matches request `id`). Gated behind
   `--features test-cdp` so it doesn't run by default.

2. **Integration** (`tools/tests/dom-smoke.ps1`):
   ```powershell
   # setup: assume task dev is up + layout-smoke creates the layout
   $auth = Get-AgentMuxAuthFile
   $layout = New-AgentMuxThreePaneLayout -Auth $auth ...
   $matches = Invoke-AgentMuxBrowserApi -Auth $auth -Method query `
       -Body @{ block_id = $layout.p1; selector = "input[name='q']" }
   if ($matches.matches.Count -lt 1) { Write-Error "no match" }
   ```

3. **Regression**: existing tests all still pass. Specifically
   `pane-focus-smoke.ps1` and `layout-smoke.ps1`.

### Estimated effort

~1-2 days hands-on. WS machinery is the bulk of it; resolver + route
are straightforward once the client works. PR commit count ~3-5.

---

## Phase 2 — remaining read endpoints

**PR shape**: now that the plumbing is proven, add the three
remaining read methods with minimal new infrastructure.

- `focus_info` — `Runtime.evaluate` on
  `document.activeElement` then re-use the `Element` serializer.
- `eval` — thin wrapper around `Runtime.evaluate` with
  `returnByValue: true` + optional `awaitPromise`. Return either the
  JSON value or the exception details.
- `screenshot` — `Page.captureScreenshot` with `format: "png"`,
  `fromSurface: true` (includes the rendered view including overlays
  the main frame would see). Returns base64.

### New files

- `agentmux-cef/src/browser_api/scripts/focus_info.js` — shares
  `__amq_path` helper from `query.js`. Extract the path helper into
  its own file and `include_str!` it into both.

### Change to existing files

- `agentmux-cef/src/browser_api/routes.rs` — three new handlers.
- `agentmux-cef/src/browser_api/types.rs` — request/response types
  for the new methods.
- `agentmux-cef/src/browser_api/mod.rs` — register new routes.

### Tests

- Extend `dom-smoke.ps1` with three new assertions: `focus_info`
  returns null before any click, returns P1's search input after
  `browser.focus_element` (Phase 3), `eval` of `1+1` returns 2,
  `screenshot` returns a byte string starting with PNG magic
  `\x89PNG`.

### Estimated effort

~half a day.

---

## Phase 3 — write endpoints

**PR shape**: the mutation surface. All four methods use existing
CDP domains; no new client infrastructure.

| Method | CDP translation |
|---|---|
| `click_element` | `DOM.querySelector` → `DOM.getBoxModel` → `Input.dispatchMouseEvent` × 2 (down/up) at the box centroid |
| `focus_element` | `Runtime.evaluate` on `document.querySelector(sel).focus()` |
| `dispatch_key` | `Input.dispatchKeyEvent` (one per key) — or `Input.insertText` for the `text:` path |
| `navigate` | Direct call to existing `browser_panes.navigate()` (bypass CDP — we already have this path) |

### Subtle bits

- `click_element` needs a real mouse event (not DOM `.click()`) so
  that `:focus-visible`, pointer-related listeners, and pane
  keyboard-focus routing all behave like a user. That's why we go
  via `Input.dispatchMouseEvent`, not `el.click()`.
- `dispatch_key` with `text:` → prefer `Input.insertText` when
  possible (atomic, doesn't need synthetic key codes). Fall back to
  per-char `dispatchKeyEvent` for single keys like `Enter`.
- `focus_element` is intentionally distinct from `click_element` —
  tests that want "give keyboard focus without synthesizing a mouse
  gesture" get a separate method.

### Tests

- Extend `dom-smoke.ps1`: click the search box, type "hello",
  read back its `value` via `eval("...")`.
- **Hook into existing tests** — the hand-off goal from the user:
  rewrite `tools/tests/pane-focus-stress.ps1` to swap pixel
  `mouse_event` + `SendKeys` for `browser.focus_element` +
  `browser.dispatch_key`. Terminal pane clicks stay pixel-based
  (terminal isn't a browser), so keep `Click-Pixel` as a fallback
  for the `Terminal` target. Coord-auto-compute can be deleted for
  the browser targets.
- Log-side invariants (`[pane-wndproc] key msg=…`, no reclaim during
  pane steps) stay, but the test no longer depends on pixel geometry
  for browser targets. All 24 stress steps should pass.

### Estimated effort

~1 day including harness rewrite.

---

## Phase 4 — harness rewrite + 24/24 pass

This phase is where the DOM API earns its keep. A single PR that:

1. Adds PowerShell wrapper helpers in `tools/tests/authfile.ps1`:
   ```powershell
   function Invoke-AgentMuxBrowserApi {
       param($Auth, [string]$Method, [hashtable]$Body)
       # POSTs to $Auth.ipc_endpoint + "/agentmux/browser/" + $Method
       # with Authorization: Bearer $Auth.ipc_token
   }
   ```
2. Rewrites `pane-focus-stress.ps1`:
   - Replace `Click-Pixel $p1.search; Send-Text 'foo'` with
     `Invoke-AgentMuxBrowserApi -Method focus_element ... ; -Method dispatch_key ...`.
   - Keep the log-side assertion (pane keystrokes must appear) — it's
     verifying a different invariant (Win32 keyboard routing) from
     what the API does (DOM state).
   - For address-bar targets, the address bar is frontend DOM, not a
     pane's DOM — those stay pixel-based or, if we want end-to-end
     DOM, add a parallel `browser.main_dom.query` against the main
     window's CDP target. Simpler: leave address-bar clicks as-is.
3. Updates `tools/tests/README.md` to document the DOM API path.

**Success criterion for this phase**: `pwsh pane-focus-stress.ps1
-CreateLayout` returns `PASS (24/24)` consistently. If it doesn't,
the `[pane-wndproc]` invariant OR the DOM API has a real bug to
fix — either way, actionable.

### Estimated effort

Half a day once Phase 3 lands.

---

## Phase 5 (optional) — connection pool

Only do this if measurement from Phase 4 shows CDP latency is a real
problem. Expected: harness run time goes from ~15 s (per-request WS)
to ~3 s (pooled). If it stays under 30 s either way, skip — the
complexity isn't worth it.

Design: `browser_api::pool::CdpPool` keyed by `block_id`. Opens on
first request, kept alive with a 60 s idle timeout, torn down on
pane close.

---

## Risk register

| Risk | Mitigation |
|---|---|
| CEF `/json` target schema doesn't expose `browserId` the way we expect | Phase 1 prototype resolver against a live dev instance before committing — curl `/json` and inspect. If `browserId` isn't present, fall back to matching `url` + `title`, or add a CEF-side identifier injection. |
| `Runtime.evaluate` injects `__amq_query` into every page the caller touches | Acceptable per spec §7. If a real problem arises, move to isolated context (`uniqueContextId` / `Runtime.executionContextCreated`). Phase 6 if needed. |
| WebSocket per request is too slow | Measure in Phase 4. Phase 5 exists for this. |
| Cross-origin iframes | Out of scope v1 (spec §2.2). Surfaces only the main frame. Add `frame_id` param when a consumer needs it. |
| Pane recreates CDP target mid-session (navigation can do this in some configs) | `resolver.rs` invalidates its cache on resolve failure, retries once. If that still fails, surface `UNKNOWN_BLOCK_ID`. |
| `tokio-tungstenite` version skew across workspace crates | Align to whatever `agentmux-srv` already uses. Check `Cargo.lock` before picking a version pin. |

---

## Dependencies

```
Phase 1  ──▶  Phase 2  ──▶  Phase 3  ──▶  Phase 4  ──▶  (Phase 5)
                                  │
                                  └─▶  Phase 3 is where the harness
                                       rewrite becomes valuable.
```

Phase 1 is the only phase with actual engineering risk — everything
after is "write another handler that calls a different CDP method".
Front-load it.

## Delivery checklist per phase

- [ ] Unit tests pass: `cargo test -p agentmux-cef --features test-cdp`
- [ ] No new warnings in `cargo check -p agentmux-cef`
- [ ] `dom-smoke.ps1` passes end-to-end against a live dev instance
- [ ] `layout-smoke.ps1` still passes (regression)
- [ ] `pane-focus-smoke.ps1` still passes (regression)
- [ ] Version bumped (`bump patch`), lockfile synced
- [ ] PR opened with explicit test-plan checkboxes + reagent LGTM
- [ ] Docs updated: `tools/tests/README.md` gains a section as new
      endpoints come online; `SPEC_BROWSER_DOM_API.md` edited only
      if a design decision changes mid-implementation
