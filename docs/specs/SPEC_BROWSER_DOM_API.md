# SPEC: Browser-pane DOM API (`/agentmux/browser/*`)

Status: draft
Date: 2026-04-19
Owner: AgentA
Motivation: pixel-coordinate clicks against browser panes are a flaky
test channel. Search box not at the expected `y`? 11/24 stress-test
failures. High-DPI monitor? All coords shift. Page re-layout after
an ad loads? Miss. We want a robust way for an external client (test
harness, automation script, future agent-driven workflow) to query
and mutate the DOM inside a running browser pane, without screen
geometry ever entering the picture.

## 1. Motivation

### 1.1 What the pane-focus stress test actually needs

The stress test asserts "after clicking P1's search box and typing
'foo', was focus correctly routed to the pane". Today it uses:

- Win32 mouse `mouse_event` clicks at a computed `(x, y)`
- SendKeys to deliver text
- Log grep for `[pane-wndproc] key msg=...` as proof-of-life

The pixel click is the weak link. Goal: replace it with something
that says "give focus to the `<input name='q'>` element in the pane
belonging to block P1", and that reports back "document.activeElement
in pane P1 is now `<input name='q'>`". No coordinates needed.

### 1.2 Other consumers

- **Agent-driven browsing**: future agents could drive web-form
  interactions programmatically instead of spinning up a separate
  headless browser.
- **Screenshot-assisted testing**: capture the pane's rendered
  content, diff against a golden image.
- **Page-load assertions**: wait for `document.readyState === 'complete'`,
  assert a title, check a meta tag.

Every case benefits from having a DOM-level RPC into the pane's
content context — neither pixel geometry nor Win32 keyboard routing
belong in the test surface.

## 2. Goals and non-goals

### 2.1 Goals

1. **HTTP-addressable DOM**: an external client can POST a selector
   and receive JSON describing matching elements — their tag, text,
   attrs, bounding rect, focus state.
2. **Input delivery**: the same client can click or type into a
   selected element without involving Win32 or the cursor.
3. **Arbitrary JS evaluation**: for cases the above don't cover,
   run a string of JS in the pane's renderer and get the serialized
   result back.
4. **Per-pane isolation**: every call is scoped to a specific
   `block_id`; one pane can't read another's DOM.
5. **Auth-gated**: same `auth_key` / `ipc_token` story as the rest
   of the API.

### 2.2 Non-goals

- Cross-origin cookie or storage manipulation (out of scope; CDP
  supports it but we don't expose it).
- Remote access. Loopback-only, same as `/agentmux/service`.
- UI recording / event replay. Out of scope.
- Subprocess / worker / iframe targeting beyond the top frame. The
  initial API exposes only the pane's main frame; add iframe
  targeting as a follow-up if needed.

## 3. Background

### 3.1 What a pane is

Each browser pane (`view: "browser"` block) is a full CEF `Browser`
instance hosted as a native Win32 child HWND inside the main window.
Managed by `agentmux-cef/src/browser_panes.rs` (`BrowserPaneManager`)
and tracked in `state.browsers` keyed by pane label.

### 3.2 What CEF already exposes

CEF ships a Chrome DevTools Protocol server on
`remote_debugging_port` — port 9223 in dev (`AGENTMUX_DEV=1` +
`main.rs:325`), 9222 otherwise. Every CEF `Browser` shows up in
`GET http://127.0.0.1:<port>/json` as a CDP target, each with its own
`webSocketDebuggerUrl` and a unique `id`.

CDP is well-documented, stable across Chromium versions, and exposes
all the primitives we need (`Runtime.evaluate`, `DOM.querySelector`,
`Input.dispatchKeyEvent`, `Page.captureScreenshot`, …).

### 3.3 What we DON'T have

- No CEF-side mapping from `block_id` → CDP target. Today the only
  correspondence between our block IDs and CEF browser IDs is
  in-process inside `BrowserPaneManager.inner.panes`.
- No client-facing endpoint that can issue CDP commands. Frontend
  never had to do this because it owns the DOM already.
- Nothing that marshals CDP JSON → our API's JSON shape.

## 4. Design options

### 4.1 Option A — CDP proxy (recommended)

Add HTTP endpoints under `/agentmux/browser/*` on the CEF IPC server.
Each handler:

1. Resolves `block_id` → CDP target id (via a new
   `BrowserPaneManager.cdp_target_for(block_id)`).
2. Opens a WebSocket to `ws://127.0.0.1:<debug>/devtools/page/<target>`.
3. Sends a CDP command, awaits the reply.
4. Translates the CDP response into our API shape, returns as JSON.

**Pros**: CDP is already there, CEF-maintained, battle-tested. No JS
injection, no renderer-side glue code. Arbitrary DOM access comes
for free (`Runtime.evaluate`).

**Cons**: Opening a WS per request is slower than a long-lived
connection (maybe ~10 ms round-trip on localhost). Easy to upgrade
to pooled WS later — start simple.

### 4.2 Option B — CefMessageRouter with injected JS

Install a `CefMessageRouter` and pre-load a script in every pane that
exposes `window.__agentmux_api(...)` backed by native handlers.
Harness would `execute_javascript` through CEF to invoke it.

**Pros**: Same-process, sub-ms overhead.

**Cons**: Requires injecting JS into every loaded page — including
google.com, third-party docs, etc. That's a real attack surface for
content that shouldn't see our API. Also: we'd have to marshal every
return value manually; V8 → Rust type bridging is error-prone.

### 4.3 Option C — `execute_javascript` + blockfile round-trip

Use CEF's one-way `execute_javascript` to run code, have that code
POST results back to `/agentmux/file` (an existing endpoint).

**Pros**: No CDP dependency.

**Cons**: Per-call: inject JS that contains a unique token, run it,
poll `/agentmux/file`. Orders of magnitude slower and more complex
than CDP. Also still requires JS injection into user-facing pages.

### 4.4 Decision

**Option A.** CDP proxy. Single-purpose, reuses CEF infrastructure,
doesn't pollute the DOM of visited pages, and returns structured
results synchronously.

## 5. API surface

### 5.1 Endpoint

Lives on the **CEF IPC server** (`ipc_endpoint` in `authkey.dev`),
not `agentmux-srv`. Reason: browser panes are managed by the CEF
host process; putting the routes there avoids an extra IPC hop.

```
POST http://<ipc_endpoint>/agentmux/browser/<method>
Header: Authorization: Bearer <ipc_token>
Content-Type: application/json
Body: { "block_id": "...", <method-specific fields> }
```

The `Authorization: Bearer` scheme is the same one already used by
the existing `/ipc` route (`agentmux-cef/src/ipc.rs:131-136`), so
the new routes reuse the existing token-validation middleware
without a new auth surface.

Response on success: `{"ok": true, "data": <method-specific>}`.
Response on failure: `{"ok": false, "error": "<message>"}`.

### 5.2 Read methods

| Method | Body | Response `data` |
|---|---|---|
| `query` | `{selector: string, limit?: number}` | `{matches: Element[]}` |
| `focus_info` | `{}` | `{focused: Element \| null}` |
| `eval` | `{script: string, await_promise?: bool}` | `{result: any, type: string}` |
| `screenshot` | `{}` | `{png_base64: string}` |

`Element` shape:

```json
{
  "selector": "input[name='q']",
  "tag": "input",
  "text": "",
  "attrs": {"name": "q", "aria-label": "Search"},
  "rect": {"x": 220, "y": 308, "width": 600, "height": 44},
  "focused": true
}
```

The `selector` returned for each match is a **derived unique path**
(backend computes it from the node position, tag, attrs), usable in
a subsequent call to target that exact node. We do NOT echo back the
caller's selector — we give them a concrete handle.

### 5.3 Write methods

| Method | Body | Response |
|---|---|---|
| `click_element` | `{selector: string}` | `{ok: true}` |
| `dispatch_key` | `{selector?: string, text?: string, key?: string}` | `{ok: true}` |
| `focus_element` | `{selector: string}` | `{ok: true}` |
| `navigate` | `{url: string}` | `{ok: true}` |

`dispatch_key` behaviour:

- If `text` is set: dispatch that text as a sequence of `keydown` +
  `char` + `keyup` events against the element (focuses it first if
  `selector` given, otherwise against the current active element).
- If `key` is set: dispatch one keystroke (CDP `Input.dispatchKeyEvent`
  with the canonical key name).

### 5.4 Auth

Every request carries `Authorization: Bearer <ipc_token>`. Missing /
wrong → 401. Same mechanism as the existing `/ipc` route, so one
middleware / extractor covers both.

### 5.5 Scope: panes, windows, instances

`block_id` is **globally unique within an `agentmux-cef` process**.
`BrowserPaneManager` is a single struct owned by `AppState`, so its
pane map spans every window in the process — tear-off windows share
one manager and one `remote_debugging_port`. The resolver therefore
doesn't need a `window_id` param: `block_id` alone is sufficient
across the full process, regardless of which window owns the pane.

Across AgentMux instances (portable A, portable B, `task dev` C),
every process has its own data dir, `authkey.dev`, IPC port, and
debug port. A harness picks the instance via `Get-AgentMuxAuthFile`
(newest live by default, `-DataDir` override for explicit selection)
and calls that instance's `ipc_endpoint`. The DOM API has no
cross-instance story — and doesn't need one, since each instance is
fully isolated by design.

**Resolver invariant**: `cdp_target_for(block_id)` asserts
single-match. If the `/json` probe surfaces zero or more than one
target pointing at our CEF `Browser.identifier()`, treat it as an
internal error (`UNKNOWN_BLOCK_ID`). The one-to-one mapping is the
contract; defending against a drifted mapping means failing fast
rather than picking arbitrarily.

### 5.6 Error conditions

- Pane closed or never existed → `{"ok": false, "error": "UNKNOWN_BLOCK_ID"}`
- Selector matched nothing → `{"ok": true, "data": {"matches": []}}` (not an error)
- CDP round-trip failed → `{"ok": false, "error": "CDP: <reason>"}`
- JS eval threw → `{"ok": false, "error": "EvalError: <message>"}` with
  stack included in `data.exception` if configured.

## 6. Implementation

### 6.1 `BrowserPaneManager` changes

Add:

```rust
/// Resolve a pane's CDP target — the page-id CEF publishes on its
/// remote debugging port. Returns None when the pane isn't Live.
pub fn cdp_target_for(&self, block_id: &str) -> Option<String>;
```

CEF exposes a browser's identifier via `browser.identifier()` (an
`i32`). Chrome DevTools Protocol target ids are strings. CEF's `/json`
endpoint lists both, so we can build a map at create time or resolve
lazily on first query.

Lazy resolution is simpler for v1: on each `cdp_target_for` call,
fetch `/json`, find the entry whose `url` matches this pane's current
URL and whose `browser_id` matches our stored identifier. Cache per
(block_id, browser_id) pair.

### 6.2 HTTP route in `agentmux-cef/src/ipc.rs`

The IPC server already uses `axum`. Add a new scope:

```rust
.route("/agentmux/browser/query", post(browser::query))
.route("/agentmux/browser/focus_info", post(browser::focus_info))
.route("/agentmux/browser/eval", post(browser::eval))
.route("/agentmux/browser/screenshot", post(browser::screenshot))
.route("/agentmux/browser/click_element", post(browser::click_element))
.route("/agentmux/browser/dispatch_key", post(browser::dispatch_key))
.route("/agentmux/browser/focus_element", post(browser::focus_element))
.route("/agentmux/browser/navigate", post(browser::navigate))
.route("/agentmux/browser/back", post(browser::back))
.route("/agentmux/browser/forward", post(browser::forward))
.route("/agentmux/browser/reload", post(browser::reload))
```

### 6.2.1 History endpoints — `back` / `forward` / `reload`

Added 2026-04-22. Agents driving a pane during dev/test need to walk the
pane's back/forward history without a human clicking the toolbar. All three
share a single request shape:

```json
POST /agentmux/browser/back     { "block_id": "…" }
POST /agentmux/browser/forward  { "block_id": "…" }
POST /agentmux/browser/reload   { "block_id": "…", "ignore_cache": false }
```

- `back` → CDP `Page.goBack`. No-op when there's no prior entry. Always
  returns `ok:true`; agents should consult the
  `browser-pane-nav-state` event (or `POST /eval { expr: "location.href" }`)
  to confirm the actual current URL.
- `forward` → CDP `Page.goForward`. Same no-op behaviour at the end of
  history.
- `reload` → CDP `Page.reload`. `ignore_cache` defaults to `false`; set
  true for the equivalent of Ctrl+F5 (bypass the http cache).

All three invalidate `target_cache.forget(block_id)` after the call (the
pane's URL changes and the next resolver probe needs to re-match). `reload`
is the exception — no URL change — but invalidating doesn't hurt.

Typical agent workflow that motivated this:

```
1. POST /navigate { block_id, url: "https://example.com/a" }
2. POST /click_element { block_id, selector: "a.link-to-b" }
3. POST /eval { block_id, expr: "location.href" }      // verify we got to /b
4. POST /back { block_id }                              // return to /a
5. POST /click_element { block_id, selector: "a.other-link" }
```

Without steps 4–5, agents had to re-navigate from scratch to explore
alternate paths.

A new `agentmux-cef/src/browser_api/mod.rs` houses the handlers.

### 6.3 CDP client

Thin wrapper in `agentmux-cef/src/browser_api/cdp.rs`:

```rust
pub struct CdpSession {
    ws: WebSocket,
    next_id: AtomicU64,
}
impl CdpSession {
    pub async fn connect(target_url: &str) -> Result<Self, CdpError>;
    pub async fn call(&mut self, method: &str, params: serde_json::Value)
        -> Result<serde_json::Value, CdpError>;
}
```

One WS per request initially. If request latency is a problem, add
a pool keyed by `block_id`.

Dependency: `tokio-tungstenite` for WS (already in Cargo.lock via
`axum`'s deps — confirm).

### 6.4 Handler sketch: `browser.query`

```rust
async fn query(State(state): State<AppState>, Json(req): Json<QueryReq>)
    -> Json<Response<QueryData>>
{
    let target = match state.browser_panes.cdp_target_for(&req.block_id) {
        Some(t) => t,
        None => return err("UNKNOWN_BLOCK_ID"),
    };
    let ws_url = format!("ws://127.0.0.1:{}/devtools/page/{}",
        debug_port(), target);
    let mut cdp = CdpSession::connect(&ws_url).await?;

    // Runtime.evaluate with a helper that returns our serialized shape.
    let js = include_str!("query.js");  // defines window.__amq_query
    let _ = cdp.call("Runtime.evaluate", json!({
        "expression": js,
    })).await?;

    let result = cdp.call("Runtime.evaluate", json!({
        "expression": format!("__amq_query({:?}, {})",
            req.selector, req.limit.unwrap_or(0)),
        "returnByValue": true,
    })).await?;

    Json(Response::ok(QueryData { matches: extract(result) }))
}
```

`query.js` is a small helper that takes `(selector, limit)` and
returns `{matches: [...]}` with the `Element` shape defined in §5.2.
Kept out of the user page's global scope by defining it on `window`
via an IIFE + gated by our token.

### 6.5 Block-id → CDP target resolution

On first query for a block_id:

1. Grab the `Browser.identifier()` from `state.browsers[label]`.
2. `GET http://127.0.0.1:<debug>/json` — returns an array of targets.
3. Match each target's `browserId` to our identifier (CEF embeds it).
4. Cache the result in `browser_api::target_cache` keyed by
   `(block_id, identifier)`.
5. On pane close (`browser_panes.close`), invalidate the cache entry.

If the match fails (pane closed underneath us, or Chromium recreated
the frame), return `UNKNOWN_BLOCK_ID` and let the caller retry.

## 7. Security

Same posture as the rest of the IPC API:

| Attacker | Defended? |
|---|---|
| Remote network | Yes — loopback-only bind. |
| Other local user | Yes — Windows ACL on data dir + `authkey.dev`. |
| Same-user process without ipc_token | Yes — token required on every request. |
| Same-user process with ipc_token | No (out of scope; they already have the key). |

**Content isolation**: `eval` runs in the pane's renderer, which may
be loaded from an untrusted origin. The helper script `query.js` is
injected into every page that gets queried; it defines a function
on `window` under a distinctive name (`__amq_*`). Pages can see it.
For a paranoid mode, v2 could use `Runtime.evaluate` with
`uniqueContextId` or `executionContextId = isolated` so the helper
lives in a separate JS world never visible to page script. Default
for v1 is the page's own world (simpler, slightly less isolated).

## 8. Test plan

- Unit: CDP client round-trip against a fake CDP server.
- Integration: new `tools/tests/dom-smoke.ps1` that opens a test
  layout, calls `browser.query` for `input[name='q']` in both
  browser panes, asserts both match and have non-empty `rect`.
- Harness rewrite: `pane-focus-stress.ps1` swaps pixel clicks for
  `browser.click_element`/`browser.focus_element` +
  `browser.dispatch_key`. The log-side invariants stay (still assert
  `[pane-wndproc] key msg=` events arrive), but the input side is
  DOM-driven.
- Regression: the existing CEF remote-debugging feature still works
  (dev ports 9223/9222 remain open; our API just layers on top).

## 9. Rollout

1. **PR 1**: this spec.
2. **PR 2**: `BrowserPaneManager.cdp_target_for` + `/json` resolver +
   one read endpoint (`browser.query`) + unit tests + smoke test.
3. **PR 3**: remaining read endpoints (`focus_info`, `eval`,
   `screenshot`).
4. **PR 4**: write endpoints (`click_element`, `focus_element`,
   `dispatch_key`, `navigate`). Port the stress test harness over.
5. **PR 5** (optional): persistent CDP connections, measured against
   per-request-WS baseline only if latency actually matters.

## 10. Open questions

- **Isolated execution context**: default to the page's world or to
  an isolated world? V1 picks page-world; revisit if it causes
  problems with CSP-strict sites.
- **WebSocket vs HTTP/2 streams**: CDP is WS-native. If axum adds
  HTTP/2 streaming and we'd rather avoid WS deps, a shim is possible
  — but premature.
- **Frame targeting**: iframes and cross-origin subframes each get
  their own target id. V1 exposes only the main frame. Spec out an
  iframe-targeting extension when there's a real consumer.

## 11. Not in scope

- `Network.*` CDP domain (request/response interception).
- `Debugger.*` domain.
- `Profiler.*`.
- Workers / service workers.
- Cross-process tracing.

Each of these is a much bigger surface and adding them isn't needed
for the stress-test use case. Hold until a concrete need appears.
