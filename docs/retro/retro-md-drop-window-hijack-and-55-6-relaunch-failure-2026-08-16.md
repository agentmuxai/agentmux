# Retro: dragging a .md file onto the 0.55.6 window destroyed the whole UI, and the relaunch afterward silently failed for a second, unrelated reason

**Date:** 2026-08-16
**Trigger:** User dragged a `.md` file onto a running AgentMux 0.55.6 window "just to see what it would do." All panes and the window's own chrome (including the close/minimize/maximize controls) were replaced by a single full-window plain-text view of the file's contents. The window would not close from within the app; the user killed it from the taskbar. Reopening 0.55.6 showed the splash screen for ~30s, then it disappeared with no window and no error.
**Status:** Root-caused from source + live logs. Two independent bugs, not one.

---

## 1. Incident A — the drop hijacked the whole window

### What actually happens on a file drop today

File drag-and-drop is deliberately scoped to individual panes, never the window as a whole:

- Terminal panes register `dragover`/`drop` listeners on their own view element (`frontend/app/view/term/term.tsx:496,498`).
- Agent panes do the same via `useAgentDropAttach` (`frontend/app/view/agent/hooks/useAgentDropAttach.ts:212,214`).
- No other view type registers a drop handler.
- There is **no window- or document-level `dragover`/`drop` listener anywhere** (checked `frontend/app-init.ts` and the whole `frontend/` tree — the only `window.addEventListener("dragover"/"drop", …)` call sites are tab-tearoff/pane-resize internals, none of which call `preventDefault()` on an arbitrary drop).

`docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md` and `docs/specs/drag-drop-files-into-panes.md` both describe this scoped-to-pane design intentionally — but neither spec (nor the code) has a fallback for a drop that lands **outside** any registered pane surface: the tab strip, the title bar, the splitters/gaps between panes, or any pane of a type that never registered a handler.

### Why that's catastrophic here, not just a no-op

A browser's default behavior for an unhandled `drop` of a local file is to **navigate the top-level frame to that file** (`file://…`, rendered as plain text for `.md`). Nothing in AgentMux blocks this for the main window:

`agentmux-cef/src/client/lifecycle.rs:762` (`on_before_browse`, CEF's `RequestHandler::OnBeforeBrowse`) is the only navigation guard in the host, and it explicitly excludes the main app window:

```rust
pub(crate) fn on_before_browse(&mut self, …) -> c_int {
    if !self.is_browser_pane {
        return 0; // main app client — never gated
    }
    …
}
```

The guard exists to stop an **embedded browser pane** from handing an external-protocol URL to `ShellExecute` (UAC-prompt risk) — it was never meant to, and doesn't, cover the main renderer navigating itself to a dropped local file.

`agentmux-cef/src/client/handlers.rs:126` (`DragHandler::on_drag_enter`) is the only other drag-related native hook, and it's deliberately non-blocking by spec: it stashes the OS file paths for the JS-side `consume_drag_paths` command and must *not* suppress the event, so the existing per-pane drop UI keeps working (`SPEC_PANE_FILE_DROP_2026_05_30.md` §3.3). It was never meant to be a navigation firewall either.

So: drop lands outside a Terminal/Agent pane → no JS handler calls `preventDefault()` → no native guard intercepts it → Chromium's default "navigate to the dropped file" behavior runs unopposed. Because AgentMux's window chrome (custom title bar, close/minimize/maximize buttons) is itself part of the same SolidJS-rendered page — not native OS chrome — that navigation wipes out the *entire* app, controls included, and replaces it with the browser's raw text rendering of the file. That is exactly what the user saw.

### Why the window wouldn't close

The in-app close button no longer existed (it was part of the DOM the navigation just destroyed). The native window itself was still alive and still responsive to the OS, which is why closing from the taskbar worked — confirmed in the srv log: the window's `HWND` was torn down *without* the normal graceful `ReportWindowClosed` handshake the JS side sends on an in-app close —

```
[ipc] WRR-DRIFT [Error] OrphanDestroy label=Some("main") hwnd=Some(2625200): HWND destroyed without preceding ReportWindowClosed for label=main
[ipc] WRR-DRIFT [Warn] OrphanInstance … Last user-visible window destroyed (crash-detected); host still alive (likely holding warm pool)
```

— i.e. srv itself flagged the close as anomalous ("crash-detected"), which lines up exactly with an external WM_CLOSE hitting a window whose JS/RPC layer had already been blown away by the navigation and so could never send its normal "I'm closing" message. The host still shut itself down cleanly a few seconds later (`CEF host exited cleanly (code 0)`), so no process got stuck — the *data* was fine, only the in-app affordance to close normally was gone.

### Fix directions (not yet implemented, filing for follow-up)

1. **Frontend, cheapest**: a single `document`-level `dragover`+`drop` listener in `app-init.ts` that calls `preventDefault()` whenever the event target isn't inside a registered pane drop-zone. Turns "drop on chrome/gaps" into a no-op instead of a navigation.
2. **Host, defense in depth**: extend `on_before_browse` (or add a `LoadHandler`) to also gate the main app client against non-`http(s)://<loopback>` navigations, or at minimum detect it and auto-recover (reload the app URL) rather than leaving the window permanently hijacked.

---

## 2. Incident B — the relaunch that silently died after ~30s

This is a **separate, unrelated bug** that would have reproduced identically even without Incident A. Traced from `~/.agentmux/logs/agentmux-launcher.log` (timestamps below are 2026-08-16, PDT; srv's own log lines are UTC, 7h ahead):

### Root cause: a schema-forward-compat guard doing exactly what it's supposed to, with no UI surfaced

0.55.6 runs under local build channel `local-main-b28b7a-cdf87dde`. That same channel's data directory was **also used by newer builds** — snapshot files on disk confirm `0.55.7` and `0.55.9` both ran against this channel more recently than 0.55.6 did:

```
snapshots/local-main-b28b7a-cdf87dde-pre-v0.55.7-2026-08-15T11-45-36Z.bak
snapshots/local-main-b28b7a-cdf87dde-pre-v0.55.9-2026-08-16T00-24-54Z.bak
```

Each of those advanced the channel's shared `objects.db` schema forward. By the time of this incident it was at **schema v21**; the 0.55.6 binary only speaks **schema v16**.

At 11:17:45 UTC, srv (0.55.6) starts, tries to open its data dir's `objects.db`, and correctly refuses — this is a deliberate, working safety guard against an older binary misreading a newer schema, not a crash:

```
WARN  database user_version is newer than this build — refusing to open. Upgrade AgentMux or switch channels. db="objects.db" found=21 expected=16
ERROR Failed to open object store: objects.db: this AgentMux is too old to open this data — schema v21 on disk, this binary speaks v16. Upgrade AgentMux, or set AGENTMUX_CHANNEL=<other> to use a fresh channel.
```

srv logs this and **keeps running** in a degraded state ("migration error (continuing)") rather than exiting — but because its object store never initializes, it never emits the `AGENTMUXSRV-ESTART` readiness line on stderr that the launcher's `spawn_srv` blocks on.

The launcher waits the full budget — **30 seconds** — with the splash window up the entire time (splash is spawned before the srv-spawn wait in `supervisor/windows.rs::run_windows`). That's the "splash for many seconds" the user saw. At 11:18:15 UTC:

```
FATAL: srv spawn failed: timeout waiting for AGENTMUXSRV-ESTART (30s)
```

`run_windows`'s handling of that specific failure (`agentmux-launcher/src/supervisor/windows.rs`, the `Err(e) =>` arm right after `srv_spawner::spawn_srv(...)`) is:

```rust
Err(e) => {
    log(&format!("FATAL: srv spawn failed: {}", e));
    eprintln!("Failed to start backend: {}", e);
    drop(job);
    std::process::exit(1);
}
```

No `show_fatal_dialog` call on this path (contrast the sibling SystemOom-budget-exhausted branch a few hundred lines later, which does call it). `drop(job)` reaps srv via `KILL_ON_JOB_CLOSE`, the process exits, and the splash window — which is just another window in that same job — disappears with it. Nothing on screen ever explains why. This matches the user's report exactly: splash up for a while, then just gone.

Confirmed via `tasklist`: no `agentmux-0.55.6.exe` / `agentmux-srv-0.55.6*` process is running now — the launcher fully tore itself down as traced above. There is no wedged/zombie instance to reconnect to; the process really did exit.

### Fix direction

Surface `show_fatal_dialog` on the srv-spawn-timeout path too, ideally including srv's own last logged ERROR line (the "too old to open this data — Upgrade AgentMux, or set AGENTMUX_CHANNEL=<other>" message already exists and is exactly what the user needs — it just never leaves the log file).

---

## 3. Is anything recoverable?

**The data itself: yes, intact.** srv refused to touch the newer-schema database rather than risk misreading it — nothing was corrupted by the version mismatch. Per `CLAUDE.md`, agent definitions/registry/transcripts are global and cross-version/cross-channel, so agent conversations and terminal history from that channel are not affected by any of this.

**The specific pane/tab layout open before the drop: no.** Two things compound here:

1. The window *did* go through a normal destroy-on-close cascade (workspace/tabs/blocks deleted) when it was closed from the taskbar — that's existing, correct behavior since PR #2186 (2026-07-16) and applies regardless of how the close was triggered.
2. The **new** pre-close snapshot mechanism that would let a relaunch restore that layout (`feat(window): restore last session's tabs/panes on relaunch`, PR #2560, `agentmux-srv/src/server/service/session_restore.rs`) only shipped in **0.55.8** (2026-08-14) — 0.55.6 (2026-08-11) predates it. So the 0.55.6 binary that actually ran the close cascade never had the code to write `Client.meta["session:last_topology"]` first. There is no snapshot to replay.

**Practical recovery:** don't relaunch 0.55.6 against this channel again — it will hit the identical 30-second-timeout-then-silent-exit every time until the channel's schema regresses (it won't) or 0.55.6 is retired. Launching that same channel with 0.55.8 or 0.55.9 (already installed, both understand schema v21) will open the database cleanly — but since no snapshot exists, it seeds the default fresh 3-pane layout rather than recovering the prior arrangement. Any agents that were open can be reopened by hand; their transcripts should still be there.

---

## 4. Sources

- `frontend/app-init.ts`, `frontend/app/view/term/term.tsx:496-498`, `frontend/app/view/agent/hooks/useAgentDropAttach.ts:212-214`
- `docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md`, `docs/specs/drag-drop-files-into-panes.md`
- `agentmux-cef/src/client/lifecycle.rs:749-785` (`on_before_browse`)
- `agentmux-cef/src/client/handlers.rs:113-176` (`DragHandler`)
- `agentmux-launcher/src/supervisor/windows.rs` (splash spawn, `spawn_srv` timeout handling, `HOST_RESTART_BUDGET`/`SRV_RESTART_BUDGET`)
- `agentmux-srv/src/server/service/session_restore.rs`, `window_close.rs`, `window_create.rs`
- `docs/retro/retro-pane-layout-restore-was-a-leak-not-a-feature-2026-08-13.md`
- `VERSION_HISTORY.md` (0.55.6 — 2026-08-11, 0.55.8 — 2026-08-14: `feat(window): restore last session's tabs/panes on relaunch`)
- Live evidence: `~/.agentmux/logs/agentmux-launcher.log` (lines ~115519–117339, dir_hash `ed4b4624df95a47e`, pid 57224, 2026-08-16 11:17:29–11:18:15 UTC), `~/.agentmux/snapshots/local-main-b28b7a-cdf87dde-pre-v0.55.{7,9}-*.bak`
