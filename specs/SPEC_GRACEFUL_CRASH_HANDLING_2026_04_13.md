# SPEC — Graceful Crash Handling

**Date:** 2026-04-13
**Status:** Draft
**Owner:** AgentA
**Trigger:** Reported 2026-04-13: VSCode and AgentMux both hit OOM; AgentMux silently turned white with no error, no recovery UI, nothing actionable for the user.

---

## 1. The problem

When something goes catastrophically wrong in AgentMux — OOM, renderer crash, sidecar exit, fatal JS error — the user currently sees **nothing**:

- The window turns white (renderer process terminated, nothing left to paint).
- No toast, no dialog, no recovery UI.
- Closing the window requires Task Manager or the title-bar close button.
- Restarting loses whatever wasn't persisted in block files.
- No clear indication *what* crashed: CEF host? sidecar? frontend JS?

For a local dev tool this is tolerable. For a daily driver that runs agent sessions for hours it isn't. Every crash becomes a silent failure plus a question ("what just happened?").

## 2. What a crash actually looks like

Four distinct failure modes, each with a different visible symptom and a different fix:

### 2.1 CEF renderer process crash (the white screen)

**Cause:** OOM in the Chromium renderer, a native-code bug inside CEF, or a JS execution panic that somehow takes out the whole V8 isolate.

**Symptom:** the window stays up (chrome, title bar, window decorations visible) but the content area turns white. No JS runs. No IPC reaches the host.

**Detection:** CEF fires `CefRequestHandler::OnRenderProcessTerminated` on the host process. It's a C++ callback available via the rust-cef bindings we already use. We do **not** currently implement it.

**What we can do:** the host still has full control — it can navigate the browser to an in-memory HTML error page, it can prompt the user, it can reload the document in-place. The sidecar is untouched so block state is preserved.

### 2.2 Sidecar (`agentmux-srv`) crash

**Cause:** Rust panic, OOM in the server process, GPU backend issue, exit code 1 on some path we missed.

**Symptom:** the WebSocket to the frontend drops. The frontend keeps running but every RPC times out and every subscribed event stops firing. Right now the UI just... sits there with stale data.

**Detection:** the WebSocket client already fires `onclose` / `onerror` events — we have them wired somewhere for reconnect logic but not for user-visible notification.

**What we can do:** show a banner at the top of the window ("Backend disconnected — reconnecting..."), attempt reconnect with backoff, show a "Restart backend" button after N failed attempts.

### 2.3 Fatal JS error in the frontend

**Cause:** unhandled promise rejection, circular dependency, a SolidJS effect that throws inside a `createEffect`, a missing import.

**Symptom:** depends on where. Sometimes the UI partially renders and then freezes. Sometimes a white screen. Sometimes a single pane goes blank while the rest works.

**Detection:** `window.onerror` and `window.onunhandledrejection` catch most of these. We already have a `showStartupError()` function in `frontend/wave.ts` but it's only called from the initial bootstrap path — **no runtime error boundary** catches failures after the app is up.

**What we can do:** install a top-level error boundary via SolidJS `<ErrorBoundary>`; install a global `window.onerror` listener; render a persistent notice ("Something broke — reload to recover") with a reload button.

### 2.4 Pane-local errors (single pane crashes)

**Cause:** a single ViewModel throws, a single agent process hits a bug, a block file gets corrupted.

**Symptom:** one pane goes blank or shows stale content; the rest of the app works fine.

**Detection:** wrap each block's `<ErrorBoundary>` around the view component.

**What we can do:** show a per-pane "This pane crashed. Reload?" placeholder that offers to re-create the view model. Other panes stay untouched.

## 3. Existing infrastructure we can reuse

- **Memory heartbeat** (`agentmux-cef/src/memory_heartbeat.rs`, v0.33.62): already logs system + per-process memory every 20s to the `mem_heartbeat` tracing target. We can surface this to the user as a warning ("memory usage at 92%, consider restarting") BEFORE a crash, not just as a post-mortem log trail.
- **WER crash dumps**: already configured per the memory file — dumps land in `%LOCALAPPDATA%\CrashDumps\` on sidecar crashes.
- **`showStartupError()`** (`frontend/wave.ts:132`): existing UI for showing a fullscreen error message with a `<pre>` block. Reusable for runtime errors with small tweaks.
- **WebSocket `onclose` / `onerror`**: already fire on backend disconnect. Need to be routed to UI, not just logged.
- **`CefLifeSpanHandler`** (`agentmux-cef/src/client.rs:467`): already implemented. Adding `CefRequestHandler` (for `OnRenderProcessTerminated`) is the same pattern.

## 4. Goals

1. **No silent white screens.** Every fatal state renders *something* the user can read, click, and recover from.
2. **Preserve work when we can.** Block state lives on disk (files under `~/.agentmux/`); a CEF renderer crash shouldn't lose it. A sidecar crash shouldn't either. Only a full machine kill loses in-flight text in the composer.
3. **One-click recovery.** The user always has a button: "Reload", "Restart", "Quit cleanly". No Task Manager required.
4. **Proactive OOM warning.** When memory usage crosses a threshold (say 85% of physical RAM OR 80% of per-process limit), warn the user *before* the crash, so they can save and restart on their terms.
5. **Telemetry trail.** Every crash writes a line to the host log with the failure mode, the memory state from the most recent heartbeat, and the last 20 log lines. Makes post-mortems actionable.

## 5. Non-goals

- **Automatic recovery.** We don't try to silently relaunch a crashed renderer; that masks bugs. The user always sees a notice.
- **Persisting composer draft text.** Out of scope for this spec. (Could be a separate follow-up.)
- **Crash reporter sending dumps to a server.** Local logs only; no network telemetry.
- **Supporting every obscure CEF failure.** Focus on the four categories in §2.

## 6. Design

### 6.1 CEF renderer-terminated handler (the biggest win)

**New file:** `agentmux-cef/src/request_handler.rs` implementing `CefRequestHandler`. Wire it from `client.rs` alongside the existing `LifeSpanHandler` / `LoadHandler` / etc.

```rust
// Sketch — not final signature
impl RequestHandler {
    fn on_render_process_terminated(
        &self,
        browser: Option<&mut Browser>,
        status: TerminationStatus,
        error_code: i32,
        error_string: Option<&CefString>,
    ) {
        let reason = match status {
            TS_PROCESS_CRASHED => "crashed",
            TS_PROCESS_OOM => "out of memory",
            TS_ABNORMAL_TERMINATION => "abnormal termination",
            TS_KILLED => "killed",
            _ => "unknown",
        };

        tracing::error!(
            target: "cef",
            status = ?status,
            error_code,
            error = ?error_string,
            "render process terminated: {}", reason,
        );

        // Log the last memory heartbeat snapshot for context
        memory_heartbeat::log_current_state("render_terminated");

        // Navigate the browser to a built-in recovery page instead of
        // leaving it white. The page is bundled into the binary at
        // compile time (include_str!) so it works even if the frontend
        // can't load.
        if let Some(b) = browser {
            b.get_main_frame()
                .expect("main frame")
                .load_url(&CefString::from(RECOVERY_PAGE_URL));
        }
    }
}
```

**The recovery page** is an `agentmux://recovery` custom-scheme URL (we already handle `agentmux://` for frontend assets) or a `data:` URL if simpler. It renders:

```
╔════════════════════════════════════════╗
║  ⚠  AgentMux hit a problem             ║
║                                        ║
║  Reason: out of memory                 ║
║  Memory at crash: 92% (14.8/16 GB)     ║
║                                        ║
║  Your open sessions are still saved.   ║
║  Reload to continue where you left off.║
║                                        ║
║  [Reload window]  [Show logs]  [Quit]  ║
╚════════════════════════════════════════╝
```

**Buttons:**
- **Reload window** → `browser.reload()` triggers a fresh navigate to the app URL; frontend bootstraps again, sidecar is untouched, sessions resume from disk.
- **Show logs** → opens `%LOCALAPPDATA%/ai.agentmux.cef.*/logs/` in Explorer via a custom protocol hook.
- **Quit** → posts a close message to the host.

The recovery page is plain HTML+CSS, no JS dependencies, bundled with `include_str!` into the host binary so it renders even if the webserver and the sidecar are both dead.

### 6.2 Sidecar disconnect banner

**New file:** `frontend/app/ui/BackendStatusBanner.tsx`. Mounts in the top-level layout (`wave.ts` or wherever the root `<div>` lives).

Reads from a new signal `backendState` (in `frontend/app/store/global.ts` or similar):

```ts
type BackendState =
    | { kind: "connected" }
    | { kind: "connecting"; since: number; attempt: number }
    | { kind: "disconnected"; since: number; lastError?: string; canRestart: boolean };
```

The WebSocket client already has `onclose` / `onerror`. Wire them to set the signal:
- First disconnect → `connecting` with exponential backoff (1s, 2s, 4s, 8s, max 30s).
- After 3 failed reconnects → `disconnected` with a "Restart backend" button.

Banner renders as a thin strip above the title bar:

```
─────────────────────────────────────
 ⚠ Backend disconnected — reconnecting (attempt 3)    [Restart backend]
─────────────────────────────────────
```

**Restart backend button:** sends a message to the CEF host via the existing IPC channel (`window.cefHost` or whatever we call it). The host re-spawns `agentmux-srv` with the same args. On success the frontend reconnects automatically.

### 6.3 Frontend error boundary + global handlers

**New file:** `frontend/app/ui/ErrorBoundary.tsx`. A SolidJS component that wraps the root tree. Renders a fallback UI when its child throws during render or effect.

**Global listeners** installed once in `wave.ts`:

```ts
window.addEventListener("error", (e) => {
    logFatal("window.error", e.error?.stack ?? e.message);
    showRuntimeError(e.message);
});
window.addEventListener("unhandledrejection", (e) => {
    logFatal("unhandled_promise", String(e.reason));
    showRuntimeError(String(e.reason));
});
```

`showRuntimeError(message)` renders an overlay on top of the app (not a full-screen takeover — the existing app might still be usable) with:
- Error message (truncated to first 500 chars)
- "Reload window" button
- "Dismiss" button (lets the user try to continue)
- "Copy details" button (for bug reports)

### 6.4 Per-pane error boundary

Wrap each `<BlockFrame>`'s view component in a SolidJS `<ErrorBoundary>`. On error, show:

```
┌──────────────────────────────────┐
│ This pane encountered an error.  │
│                                  │
│ Error: <short message>           │
│                                  │
│ [Reload pane]  [Close]  [Logs]   │
└──────────────────────────────────┘
```

**Reload pane:** disposes the current view model and re-creates it from the block's saved meta. Works because view models are all stateless-on-meta by design.
**Close:** removes the pane from the layout.
**Logs:** opens the logs directory.

### 6.5 Proactive OOM warning

Memory heartbeat fires every 20s. Add a threshold check:

- **System physical memory load > 85%:** emit a `mem_warning` tracing event with severity `warn`.
- **Any AgentMux process (host/srv/renderer) > 80% of per-process 4GB limit:** emit with severity `error`.

The host exposes these events over IPC to the frontend via a `system:memory_warning` wave event. A new `MemoryWarningBanner` component renders a thin orange strip:

```
⚠ Memory usage 89% — consider restarting AgentMux before it crashes   [Restart]
```

The banner auto-dismisses when memory drops below 80%.

### 6.6 Crash context in logs

Every crash path writes one structured log line via the existing `tracing` pipeline:

```rust
tracing::error!(
    target: "crash",
    kind = "renderer_oom",
    memory_pct = 92,
    last_heartbeat = %last_heartbeat_json,
    last_20_log_lines = %recent_logs_json,
    "crash detected",
);
```

The `last_heartbeat_json` is captured from the memory heartbeat module's most recent sample. The `last_20_log_lines` is captured from an in-memory ring buffer that `tracing`'s subscriber already has access to (need to add a small `recent_logs` layer).

## 7. Files to add / modify

### New files

- `agentmux-cef/src/request_handler.rs` — `CefRequestHandler` impl with `on_render_process_terminated`.
- `agentmux-cef/assets/recovery.html` — bundled static HTML for the recovery page (or generated inline).
- `agentmux-cef/src/recent_logs.rs` — small ring buffer `tracing` layer keeping the last N log entries in memory.
- `frontend/app/ui/BackendStatusBanner.tsx` — disconnect banner + restart button.
- `frontend/app/ui/MemoryWarningBanner.tsx` — proactive OOM warning.
- `frontend/app/ui/RuntimeErrorOverlay.tsx` — overlay for `window.error` / `unhandledrejection`.
- `frontend/app/ui/PaneErrorBoundary.tsx` — per-pane SolidJS `<ErrorBoundary>` wrapper.

### Modified files

- `agentmux-cef/src/client.rs` — wire the new `RequestHandler`.
- `agentmux-cef/src/memory_heartbeat.rs` — add threshold check, emit `mem_warning` event.
- `agentmux-cef/src/main.rs` — register the `recent_logs` tracing layer.
- `frontend/wave.ts` — install global `error` / `unhandledrejection` listeners, wire `backendState` signal to WebSocket lifecycle.
- `frontend/app/store/global.ts` — new `backendState` atom.
- `frontend/app/block/blockframe.tsx` (or wherever block view components mount) — wrap in `<PaneErrorBoundary>`.
- `frontend/app/store/websocket.ts` (or current WS client location) — emit backend-state transitions on connect/close/error.

## 8. Implementation order (incremental, each shippable)

1. **CEF render-process-terminated handler + recovery HTML page.** Highest user-facing impact. Fixes the white screen directly.
2. **Frontend `<ErrorBoundary>` + global `onerror` / `onunhandledrejection`.** Catches JS-layer failures that aren't renderer crashes.
3. **Per-pane `<ErrorBoundary>`.** Prevents single-pane failures from taking out the whole app.
4. **Backend disconnect banner + reconnect logic.** Fixes the stale-UI-after-sidecar-crash case.
5. **Memory warning banner.** Proactive; ships after the reactive fixes so we know what a typical memory timeline looks like.
6. **Crash context logging (recent_logs ring buffer).** Diagnostic; nice-to-have once the user-visible pieces are in place.

Each step is one PR. The first two fix the reported problem entirely.

## 9. Estimated cost

| Step | Files | Est. time | Review rounds |
|---|---|---:|---:|
| 1. CEF RequestHandler + recovery HTML | 3 new, 1 modified | 2 h | 1–2 |
| 2. Frontend ErrorBoundary + global handlers | 2 new, 1 modified | 1 h | 0–1 |
| 3. Per-pane ErrorBoundary | 1 new, 1 modified | 45 min | 0–1 |
| 4. Backend disconnect banner | 1 new, 2 modified | 90 min | 1 |
| 5. Memory warning banner | 1 new, 2 modified | 60 min | 0–1 |
| 6. Crash context logging | 1 new, 1 modified | 45 min | 0–1 |

**Total:** ~6.5 hours of coding, 2–7 review rounds.

## 10. Risks and open questions

### 10.1 `OnRenderProcessTerminated` bindings

Need to verify our `cef-rs` binding crate exposes `RequestHandler::on_render_process_terminated`. If not, we may need to add it or use a different hook. Check with a grep before scheduling step 1.

### 10.2 Recovery page URL scheme

We already use `agentmux://app/...` (or similar) for the bundled frontend. Check if the custom scheme handler can serve a second route `/recovery.html` from an `include_str!` blob, or if we need a `data:` URL. `data:` URLs work everywhere and don't require scheme handler changes — probably the simpler path.

### 10.3 Reload button vs. navigate

When the user clicks "Reload", `browser.reload()` might re-use the crashed renderer context. Need to verify. If so, use `load_url(original_url)` instead to force a fresh load.

### 10.4 Banner real estate

Three possible top-of-window banners (backend disconnect, memory warning, runtime error overlay). Stack them? Show one at a time with a priority? Start with a simple vertical stack and see if it gets ugly in practice.

### 10.5 Sidecar restart UX

The "Restart backend" button calls the CEF host to re-spawn `agentmux-srv`. What happens to any open block controllers that were mid-run when the sidecar died? Probably they're just gone and the user starts fresh turns. Document this limitation in the banner text.

### 10.6 Error boundary and SolidJS HMR

In dev mode with Vite HMR, a file edit triggers hot reload which might look like an error to `<ErrorBoundary>`. Verify the boundary doesn't fire spuriously during dev. If it does, gate the boundary on `import.meta.env.PROD`.

## 11. Success criteria

After all 6 steps:

- **CEF renderer OOM / crash:** the window shows a recovery page with reload/quit/logs buttons instead of going white.
- **Sidecar exit:** a banner appears at the top of the app within 2 seconds; the user can click Restart Backend to re-spawn the sidecar without restarting the host.
- **Unhandled JS error:** an overlay appears with the error message and a reload button; the rest of the app stays interactive if possible.
- **Single pane crash:** only that pane shows an error card; other panes keep working; the user can reload the pane to recover.
- **Memory warning:** at 85% system memory or 80% per-process, an orange banner appears offering to restart the app before the inevitable crash.
- **Every crash is diagnosable:** the host log shows the failure mode, memory state, and last 20 log lines without the user having to ask.

## 12. Out of scope

- **Persisting in-flight composer text to disk.** Separate spec if worth it.
- **Crash reporter backend for telemetry.** Local logs only.
- **Process isolation per pane** (Chromium's site isolation for different blocks). Too invasive; relies on CEF internals.
- **Automatic sidecar restart on every disconnect.** We show a button; user decides. Silent relaunch masks real bugs.
- **Improving WER crash dump quality.** Already configured.
- **Reducing OOM root causes** (document trimming, heap limits in V8, smarter content-visibility). Separate work; orthogonal to "when it crashes, handle it gracefully."

## 13. Relationship to existing work

- **PR #222 (0.32.78)** — FileStore cache OOM fix. Addresses one root cause. This spec addresses the *symptom* when any OOM occurs.
- **Memory heartbeat (0.33.62)** — logs a post-mortem timeline. This spec surfaces it to the user proactively and in crash recovery.
- **WER crash dumps** — already in place for sidecar native crashes. This spec complements the dump with a user-visible notice.
- **Startup error UI** (`showStartupError` in `wave.ts`) — already exists for bootstrap failures. This spec extends the same visual treatment to runtime failures.

Nothing in this spec conflicts with shipped work. It's all additive: new handlers, new banners, new error boundaries on top of a stable foundation.
