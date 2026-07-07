# RETRO — browser-pane close reports success but the CEF renderer survives (task #1 investigation)

**Date:** 2026-07-07
**Author:** AgentA
**Severity:** High — a currently-reproducible renderer-process leak, distinct from (not fixed by) PR #1957 and its Round 2 follow-up. Every browser-pane open→close cycle appears to leak one renderer process indefinitely.
**Area:** `agentmux-cef/src/browser_panes.rs` (`close`, `close_with`), `agentmux-cef/src/browser_pane/wrapper.rs` (`destroy_wrapper_hwnd`, `wrapper_wndproc`).

---

## Summary

Task #1 asked to independently confirm the one item PR #1957's own test plan flagged as
**"not independently re-confirmed"**: renderer-process-count returning to baseline after repeated
browser-pane open→close cycles. Live-tested on an isolated instance (`agentmux-0.50.3+g2f698fb7`,
which already includes PR #1957's fix and its Round 2 follow-up, task #11).

**Result: it does not return to baseline, and the leaked renderer is genuinely alive, not a
process-teardown-lag artifact.**

## Method

1. Baseline renderer-process count (Win32 `CommandLine like '%type=renderer%'` filtered to the test
   instance's executable path): **5**.
2. 4× cycles of `POST /api/v1/pane/open {view: "browser", url: "https://agentmux.ai", tab_id}` →
   wait 3s → `object.DeleteBlock(block_id)` (the same RPC the frontend's pane-close button calls) →
   wait 3s.
3. Host log confirms **all 4 cycles completed the exact success sequence PR #1957's own test plan
   describes**: `[pane-wrapper] created` → `BrowserUnregistered` → `[pane-wrapper] destroyed` →
   `browser pane closed`, e.g.:
   ```
   [pane-wrapper] created label="browser-pane-38593e0c-...-3" hwnd=0x2e06dc
   ...
   event="BrowserUnregistered" label=browser-pane-38593e0c-...-3 version=32
   [pane-wrapper] destroyed hwnd=0x2e06dc
   pane HWND destroyed label="browser-pane-38593e0c-...-3"
   browser pane closed block_id="38593e0c-..." label="browser-pane-38593e0c-...-3"
   ```
4. Renderer-process count immediately after all 4 cycles: **9** (baseline 5 + 4, matching the 4 panes
   opened, none released). Polled every 10s for a further 60s — **stable at 9 the entire time**, not a
   transient teardown lag.
5. **Decisive confirmation, not inference:** queried the host's CDP `/json/list` after all 4 "closes" —
   found **4 separate targets still listed with `url: "https://agentmux.ai/"`**. Connected to one via
   its own `webSocketDebuggerUrl` and ran `Runtime.evaluate` — it returned
   `"AgentMux: Agent Operating Environment|https://agentmux.ai/"` immediately. **The page is genuinely
   alive and responsive**, not a stale directory-listing artifact — its own renderer process is still
   fully live, invisible (no wrapper HWND, no window), running indefinitely.

## Root cause (code-level, not yet fixed)

`browser_panes.rs::close()`/`close_with()` and `browser_pane/wrapper.rs::destroy_wrapper_hwnd()` are
built entirely on an **assumed, unverified** mechanism (per both files' own doc comments): destroying
the wrapper's HWND via Win32 `DestroyWindow` is expected to cascade via `WM_DESTROY` down to CEF's
embedded child HWND, which is expected to reliably trigger CEF's internal `OnBeforeClose` — "per the
floater's already-proven behavior" (the comment's own phrase, `browser_panes.rs:401-402`,
`wrapper.rs:20-22,254-256`).

**Nothing in this path actually confirms `OnBeforeClose` fired, or that the CEF `Browser` object's
refcount reached zero, before declaring success.** `close()` dispatches `CompleteBrowserPaneClose`
and logs `"browser pane closed"` (`browser_panes.rs:452-457`) immediately after the synchronous
`DestroyWindow` call returns — there is no wait, no callback confirmation, no fallback if the cascade
doesn't actually tear down CEF's side. My test shows this optimism is unfounded: for all 4 of my test
cycles, the wrapper HWND destroy completed and the reducer's bookkeeping declared victory, but CEF's
underlying `Browser` object (and its renderer process) survived intact and fully functional.

This is a **different failure mode from the one PR #1957 fixed**: that PR's bug was CEF's
`on_before_close` "only 'may eventually' fire" when destroying CEF's own HWND directly — the wrapper
indirection was meant to make that firing reliable, and per the reducer's bookkeeping it now *appears*
to fire every time (`BrowserUnregistered` logs correctly). What this retro found is that **even when
the host believes the browser closed (and the events/logs agree), the actual CEF/Chromium
renderer process underneath does not necessarily exit** — i.e. either `on_before_close` isn't
genuinely firing despite the log evidence looking like it did, or it fires but something (a lingering
`Browser` Arc reference, a CEF-internal keep-alive, a site-isolation/process-reuse quirk) keeps the
renderer process alive regardless.

## Why this wasn't caught by PR #1957 / task #11

Both prior fixes' test plans checked the **event/log sequence** (`on_before_close` firing, no
main-window cascade, no deadlock) — exactly the symptoms their own bugs produced. Neither checked the
underlying OS **process** count against a live CDP target list, which is the only way this retro's
finding actually surfaces (the reducer-level bookkeeping looks completely healthy; you have to go
around it, to the actual OS/CDP layer, to see the leak).

## Action items

1. **Do not trust `BrowserUnregistered`/"browser pane closed" as proof the renderer exited.** Any
   future renderer-count verification (including a proper e2e test for task #9) must check the actual
   OS process count and/or CDP target list, not just the host's own event log.
2. **Root-cause why the wrapper's `WM_DESTROY` cascade doesn't reliably tear down CEF's embedded
   child in practice**, despite the doc comments' confident claim that it mirrors the floater's
   "already-proven" behavior. Candidates worth checking first: whether the CEF `Browser` Arc
   (`close_with`'s doc comment says "Drop our `Browser` Arc" but the code excerpt read for this retro
   doesn't show where that drop actually happens relative to the HWND destroy — verify the ordering
   and that nothing else holds a clone); whether Chromium's spare-renderer-process pre-warming is
   confusing the count (ruled out here by the CDP-target-alive check — a spare/pre-warmed renderer
   would not still be attached to a live page at the pane's specific URL); whether `close_browser()`
   (explicitly bypassed by this design, per the doc comment's own trade-off note about `beforeunload`)
   is actually required for CEF to release the renderer, contrary to the floater precedent this
   design leaned on.
3. **This is exactly the class of "renderer/pagefile growth" the broader OOM investigation
   (`SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`) has been circling all week** — cross-reference
   directly (this retro's mechanism is a strong, concrete, reproducible candidate contributor,
   distinct from the main-window/pool-window leaks already found and fixed this session).
4. **Add the e2e test task #9 already calls for**, but written against the CDP-target-alive check
   this retro used to actually catch the bug — a test that only checks host log lines or the
   reducer's own browser-count would have passed throughout this entire investigation.

## Reproduction (for whoever picks this up)

```
POST http://<web_endpoint>/api/v1/pane/open
  {"view": "browser", "url": "https://agentmux.ai", "tab_id": "<active tab>"}
  → returns { "block_id": "..." }

POST http://<web_endpoint>/agentmux/service
  {"service": "object", "method": "DeleteBlock", "args": ["<block_id>"], "uicontext": null}

# Host log shows a clean create/destroy/closed sequence. But:
GET http://<cdp_debug_port>/json/list
  → still lists a target with url: "https://agentmux.ai/" for this pane
  → connect to its own webSocketDebuggerUrl, Runtime.evaluate → responds normally, fully alive
```
No special timing or concurrency needed — reproduced on the very first attempt, and on all 4
sequential (non-overlapping) cycles in this session's test.
