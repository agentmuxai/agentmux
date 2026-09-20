# SPEC: Deep, no-mouse control of a browser pane via the Agent App API

**Date:** 2026-09-20
**Status:** implemented — PR #3445.
**Related:** `docs/specs/SPEC_AGENT_BROWSER_CONTROL_2026_04_17.md` (original
2026-04-17 proposal — Draft, its tool names never built, superseded in
spirit by this doc's design against the architecture that actually shipped),
`docs/specs/SPEC_BROWSER_DOM_API.md` (implemented, PR #453 — delivered the
CDP-backed control layer this spec exposes, explicitly scoped agent-driven
use as future work), `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`
(implemented — the own-pane-only scoping precedent this spec follows).

---

## 0. The ask

"Deep control of the browser" for an agent that has a browser pane open —
not synthetic OS-level mouse/pixel automation (the CDP-over-screen approach
used earlier this session for whole-window screenshot tooling), but
structured, semantic control reachable through the Agent App API: navigate,
read page content, click by selector, type, all without moving a literal
mouse. Question: do we already have this?

## 1. What exists today

**The semantic control layer already exists and is fully built.** Every
browser pane is a genuine, separate CEF `Browser` instance (not an iframe —
`agentmux-cef/src/browser_pane/creation.rs`), and `agentmux-cef/src/browser_api/`
(`routes.rs`, `resolver.rs`, `cdp.rs`) is a CDP-backed HTTP control surface
with 11 routes: `query`, `focus_info`, `eval`, `screenshot`, `click_element`,
`focus_element`, `dispatch_key`, `navigate`, `back`, `forward`, `reload`.
This is exactly the "selector-based, no-mouse-required" layer being asked
about — `click_element` still dispatches a synthetic `Input.dispatchMouseEvent`
under the hood, but computed from a CSS selector, not a literal input
device.

**Only 3 of the 11 routes reach an agent.** `UIClick`/`UIQuery`/`UIScreenshot`
(`agentmux-mcp`, proxied through `agentmux-srv/src/server/ui_handlers.rs`)
map straight onto `click_element`/`query`/`screenshot`. The other 8 —
`navigate`, `back`, `forward`, `reload`, `eval`, `dispatch_key`,
`focus_element`, `focus_info` — exist in `browser_api` but have no App-API
proxy and no MCP tool. They're reachable today only by whatever holds the
CEF host's internal `ipc_token` directly (an internal test harness), not by
an agent.

**Scoping precedent: own pane only, enforced server-side.** `verified_block_id()`
derives `block_id` from the caller's HMAC-signed agent identity —
a client-supplied `block_id` is never trusted (PR #2662, a CVE-style fix
against cross-pane targeting). So even the 3 exposed tools can only ever
touch the calling agent's *own* pane. There is no way today for an agent to
drive a browser pane it doesn't itself own.

## 2. The gap, precisely

Two distinct gaps, not one:

1. **Exposure gap** — 8 built routes have no MCP tool. This is the literal
   answer to "do we have that": partially. Click/query/screenshot, yes.
   Navigate, type, run JS, read focus state, history — no.
2. **A latent safety gap in the unexposed routes themselves**, found while
   tracing the resolver (`agentmux-cef/src/browser_api/resolver.rs`):
   `resolve()` returns `ResolvedTarget { target_id, scope_to_block }`, where
   `scope_to_block` distinguishes a **Path 1** target (a dedicated
   browser-pane's own isolated CEF page — third-party content, safe to eval/
   navigate freely) from a **Path 2** target (an agent/terminal/editor pane,
   which is just a DOM node inside a page *shared* with the whole
   window's chrome and every other pane in it).

   `click_element`, `focus_element`, and `dispatch_key` all correctly branch
   on `scope_to_block` to scope their DOM operations to `[data-blockid]`
   when the target is shared. **`eval`, `navigate`, `back`, `forward`, and
   `reload` all discard it** (`let (mut cdp, _scope_to_block) = ...` —
   `routes.rs:228,712,750,782,814`) and act on the resolved CDP target
   unconditionally. Concretely: `eval`'s own doc comment already admits "the
   script runs in the pane's JS world (not an isolated context); treat it
   as arbitrary code execution in whatever origin the pane currently loads"
   — for a Path 2 block, "whatever origin the pane currently loads" is the
   **entire shared AgentMux window**, not a sandboxed page. And `navigate`
   calling CDP `Page.navigate` against a Path 2 target would navigate the
   *entire shared window* away, for a `block_id` that isn't even a browser
   pane.

   This is unreachable by an agent today (no MCP tool calls these routes),
   so it isn't an active vulnerability — but it means naively wiring these
   8 routes straight into MCP tools, unchanged, **would** turn "agent
   controls its own browser pane" into "agent can eval arbitrary JS in, or
   navigate away, the shared window hosting every other pane in the
   session" the moment the agent's own pane happens to resolve via Path 2
   (i.e., whenever the calling agent's *own* pane isn't itself a browser
   pane — the common case, since most agents' own panes are terminal/chat
   panes, not browser panes).

## 3. Proposed design

### 3.1 Fix the routes first (precondition, not optional)

Before any new MCP exposure, `agentmux-cef/src/browser_api/routes.rs`'s
`eval`, `navigate`, `back`, `forward`, and `reload` must reject a Path 2
(`scope_to_block == true`) target with a clear error (e.g. `"block <id> is
not a browser pane — eval/navigate/back/forward/reload require a dedicated
browser-pane target"`) instead of silently acting on the shared window.
This closes the gap in §2.2 regardless of what calls these routes next,
and makes the MCP tools below safe by construction rather than by caller
discipline.

### 3.2 New App-API / MCP tools, own-pane-only (same precedent as UIClick/UIQuery/UIScreenshot)

| Tool | Proxies | Notes |
|---|---|---|
| `BrowserNavigate(url)` | `navigate` | Rejects if caller's own pane isn't a browser pane (post-3.1 fix). |
| `BrowserBack()` / `BrowserForward()` / `BrowserReload()` | `back`/`forward`/`reload` | Same. Ack-only; agents confirm via `BrowserEval("location.href")` or existing nav-state events. |
| `BrowserEval(expression)` | `eval` | Same. Returns the CDP-serialized value or a thrown-exception message. This is the highest-leverage tool — read page text (`document.body.innerText`), fill forms (`el.value = ...; el.dispatchEvent(...)`), wait for app-specific state, all without a selector-by-selector API for every possible action. |
| `BrowserDispatchKey(key, modifiers?, text?)` | `dispatch_key` | Types into whatever currently has focus — the actual "without a mouse" input path for text fields. Kept scope-aware like the existing `click_element`/`focus_element` (safe for both Path 1 and Path 2, since it doesn't execute code, only synthesizes a key event CDP already scopes correctly today). |
| `BrowserFocusElement(selector)` / `BrowserFocusInfo()` | `focus_element`/`focus_info` | Already correctly scope-aware in `routes.rs`; wire through unchanged. |

All six follow the existing `UIClick`/`UIQuery`/`UIScreenshot` pattern
exactly: `agentmux-mcp` tool → `agentmux-srv/server/ui_handlers.rs` proxy →
`verified_block_id()` (server-derived from caller identity, never
client-supplied) → `browser_api` route. No new trust boundary, no new
resolver logic — this is wiring plus the §3.1 fix, not new architecture.

### 3.3 Tool descriptions must say what they can't do

Following the existing precedent (`tool_schemas.rs` already says
`UIClick`/`UIQuery` "cannot reach a DIFFERENT pane or agent's UI"),
`BrowserNavigate`/`BrowserEval`/etc.'s descriptions must state plainly: (a)
own-pane only, (b) `BrowserNavigate`/`Back`/`Forward`/`Reload`/`Eval` only
work when the caller's own pane is itself a browser pane — an agent whose
own pane is a terminal/chat pane gets a clear rejection, not a silent
no-op or (pre-fix) a dangerous fallback to the shared window.

## 4. Explicitly out of scope

- **Controlling a browser pane the calling agent doesn't own** (e.g. a
  supervisor agent driving a subordinate's or a shared window's browser
  pane). Blocked by the same `verified_block_id()` design that scopes
  `UIClick`/`UIQuery`/`UIScreenshot` today, for the same reason (PR #2662).
  This is a bigger, separate design problem — it needs its own
  authorization model (who may grant cross-pane access, to whom, for how
  long) — not an extension of this spec's own-pane wiring.
- **Changing `BrowserPaneManager`'s frontend IPC surface**
  (`browser-model.ts`'s `navigate`/`goBack`/etc.) — those are the human
  user's own UI-driven navigation controls and are unaffected; this spec
  adds a parallel agent-facing path through the existing CDP-backed
  `browser_api`, not a change to how the pane's own chrome works.
- **A `BrowserGetText`/`BrowserGetHTML` dedicated tool** — deliberately
  folded into `BrowserEval` (`eval("document.body.innerText")` etc.)
  rather than adding single-purpose tools for every read pattern; keeps
  the tool surface small the same way the original 2026-04-17 draft's
  larger tool list was avoided.

## 5. Platform scope

Entirely backend (`agentmux-cef`/`agentmux-srv`) and CDP-based — no
`cfg(windows)`/`cfg(unix)` branching anywhere in `browser_api`. Applies
identically on every platform `task dev`/packaged builds run on.

## 6. Open follow-ups

- Whether `BrowserDispatchKey` should also gate on `scope_to_block` for key
  combinations that could trigger app-level shortcuts when sent to a Path 2
  (shared-window) target — not investigated here; flagged for whoever
  implements §3.2 to check against the app's actual keybinding surface
  before shipping.
- The cross-pane authorization model from §4 — worth its own spec once
  there's a concrete use case (e.g. an orchestrating agent that needs to
  read a browser pane it spawned in another agent's tab), not before.
