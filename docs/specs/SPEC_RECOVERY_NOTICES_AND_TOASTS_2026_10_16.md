# Spec: tell the user what happened after AgentMux recovers, and fix the toasts that carry it

**Date:** 2026-10-16
**Status:** active. §6's cause fixed in #5024; §4.5 (toasts that show in every build) in #5030; §4.1 to §4.4 proposed.
**Author:** Loap-3@narko, at the operator's request
**Scope:** the frontend's toasts (`frontend/app/notification/`, `store/flash-notifications.ts`), the status bar's backend item (`statusbar/BackendStatus.tsx`, `HealthNotices.tsx`), the host's crash and quit paths (`crates/cef/src/client/crash_recovery.rs`, `client/lifecycle.rs`, `wrr/`, `lib.rs` fast exit), the launcher's supervisor (`crates/launcher/src/supervisor/`), srv's start-up recovery (`blockcontroller/session_recovery.rs`, `service/session_restore.rs`).
**Related:** `SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md` (the renderer recovery page), `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` (crash budgets), `SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`, `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md`, `SPEC_FLEETWIDE_CRASH_HANG_FORENSICS_2026_08_14.md`, `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §6 (the background audit), `docs/reports/REPORT_PERFORMANCE_INDICATORS_TO_STATUS_BAR_2026_10_08.md`.

## 1. The ask

From the operator, 2026-10-16, after an instance vanished with no explanation:

> in cases like this, we need to toast the user after recovery with what happened, including what AgentMux did to save your work. Whether its GPU, or chromium, or whatever it is, put it in a toast, and dont auto-close it. it requires a manual close. … we also want to look at the overall toasts systems. What toasts do we currently have? should this go in the same place as the resource warnings, in the status bar uptime status?

Three things, then: a recovery notice that names the cause and what was saved and stays until closed; a look at the toasts AgentMux has; and where the notice lives.

## 2. The incident that prompted it

On 2026-10-16 at 13:42 UTC, an AgentMux 0.59.26 instance disappeared while the operator was signing in to GitHub in a browser pane (the "connect GitHub" step of a claude.ai connector):

| UTC | What the logs show |
|---|---|
| 13:42:16 | The pane loads `claude.ai/connect/github/callback`, the OAuth callback page. |
| 13:42:21.13 | The operator clicks in it. |
| 13:42:21.29 | The pane closes, then the window's other two browser panes. The host sees **0 visible windows**: the main window is gone. |
| 13:42:21 to :33 | The quit watchdog (`wrr`): "0 visible for 3000ms … re-arming (1/3)", (2/3), (3/3). |
| 13:42:33 | "CEF message loop exited, shutting down … AgentMux host shutdown complete (fast exit)". |

**Cause.** The callback page called `window.close()`, as OAuth callbacks do when they expect to be a popup. `do_close` (`crates/cef/src/client/lifecycle.rs:715`) allows the close for every browser, panes included, and for a windowed browser CEF answers an allowed script close by sending `WM_CLOSE` to the browser's top-level window: the AgentMux main window. So a web page closed the instance; the 12 seconds before the watchdog quit showed the window's bare background, the AgentMux logo, which the operator saw as "blank with a logo in the centre".

Nothing told the operator any of this. The instance was gone; nothing on the next launch said it had quit, why, or that its agents' conversations and layout were safe. (The cause is fixed: §6.)

## 3. What AgentMux has today

### 3.1 In-app toasts don't show in a release build

`pushNotification` (`store/flash-notifications.ts:29-33`) adds to a list that only `<NotificationBubbles />` renders, and that is mounted inside `<Show when={isDev()}>` (`frontend/app/app.tsx:469-471`). `isDev()` is true only in a `task dev` build. The gate has been there since the SolidJS migration (2026-03-13). **In every packaged build, every toast below is dropped silently.** Only `pushFlashError` (two callers, both "couldn't close the pane") shows in release.

The toasts that are being dropped (23 call sites in 21 files):

| Kind | Examples |
|---|---|
| A UI action failed | "Rename failed", "New tab failed", "Delete failed", a quick-fork failure, an editor action failure |
| Drop, attach, paste | drop failed or copied, "Paste too large", "Pasting…" progress |
| Layouts | saved, opened, failed |
| Confirm or undo | "<widget> hidden" with **Undo**; "<agent> quit" |
| Devices | "Device paired" |
| While you were away | "Ran while you were away" (`init/background-audit.ts:57`, no expiry) |
| Widgets | the widget SDK's `ctx.toast` |
| Voice | microphone and transcription errors |

The toast system also has gaps that a sticky recovery notice would hit:
- The X only **hides** a toast (`hidden: true`, `notificationbubbles.tsx:53`); hidden toasts stay in the list for good.
- No cap or queue: a burst of toasts stacks without limit.
- `persistent` is accepted but the lock icon renders only in a variant nothing mounts (`notificationitem.tsx:40`).
- One built-in action (`installUpdate`), which nothing calls; others are registered at runtime.

### 3.2 Notices that do reach the user

- **OS toasts** (Windows, Linux; macOS not built): srv's notify Router decides (`backend/notify/router.rs`, kinds in `policy.rs:23-42`: a question, turn done or failed, an agent crashed, a review, a pending shutdown, a summary, cloud signed out), and the launcher shows them.
- **"Waiting for you"**: `user_attention` (consent, browser, widget, upgrade requests) → the waiting tone, an OS toast, the taskbar badge.
- **The status bar's backend item** (`BackendStatus.tsx`): a dot and the uptime. The dot turns yellow while any health signal is active (RAM or page-file pressure, a slow backend; `store/health-signals.ts`, from the host's `health_signals.rs`) and red when srv has died; clicking it opens a popover with `HealthNotices` (worst first, "for N min"), srv's status, and, after a crash, "Died at" and **Restart Backend**. No toast is raised for any of these.
- **The renderer recovery page** after a renderer crash ("AgentMux hit a problem", Reload/Quit, `client/crash_recovery.rs`).
- **Launcher dialogs** (Windows only): the out-of-memory give-up, a failed migration, "already running but not responding".

### 3.3 After a recovery, nothing

| What happened | Who handles it | What is recorded | What the user is told afterwards |
|---|---|---|---|
| Renderer crash, OOM or hang | host `crash_recovery.rs` | a `crash` log line | the recovery page; after Reload, nothing |
| GPU process crash | Chromium restarts it | a log line | nothing |
| srv exits (launcher mode) | launcher respawns srv, recycles the host | launcher log | "Restoring session…" splash, then nothing |
| Host exits abnormally | launcher restarts it (3 per 60 s; steps down to `--disable-gpu` when the same exit repeats) | launcher log | splash, then nothing |
| Host hangs | launcher | a minidump, a log line | nothing |
| No window left (the incident above) | quit watchdog, then fast exit | `[wrr]` log lines | nothing, ever: the app is gone |
| Agents cut off by srv dying | `session_recovery.rs` | `session:was_interrupted` | a banner inside each pane |
| Session restored on relaunch | `session_restore.rs` | the snapshot | the layout comes back, silently |

Nothing durable records why AgentMux last stopped or what it recovered from, so nothing can say so afterwards.

## 4. Design

### 4.1 An incident record

Every path in §3.3 appends one record to an **incident log**: one JSON line per incident in the instance's data folder (`<instance>/incidents.jsonl`), as the background audit does (`crates/cef/src/background_audit.rs`). Host, launcher and srv each write their own cases; the file is append-only and capped (the last 200 records).

```json
{ "id": "…", "at_ms": 0, "kind": "window_closed_by_page", "process": "host",
  "what": "A web page in a browser pane closed the AgentMux window.",
  "cause": "claude.ai called window.close() after the GitHub sign-in.",
  "did": ["Saved your layout", "Kept every agent's conversation; reopen them from My Agents"],
  "lost": [],
  "evidence": [{ "log": "agentmux-host-v0.59.26.log.2026-10-16", "at": "2026-10-16T13:42:21Z" }],
  "acknowledged_at_ms": null }
```

- `kind`: a closed set (§4.4).
- `what`, `cause`: plain words for the user. `cause` is left out when it isn't known; it is never guessed.
- `did`: what AgentMux did that protects the user's work, each a fact the writing path knows (it saved the layout, respawned srv, restored N panes, marked M agent turns interrupted so they resume). `lost`: what may not have survived, when the path knows (an unsent message, a running command in a terminal).
- `evidence`: log file and time, for a bug report.
- `acknowledged_at_ms`: set when the user closes the notice (§4.2).

The writer is the process that handled the incident, at the time it handled it: the host before its fast exit, the launcher when it restarts something, srv when its start-up finds orphaned turns. A fast exit writes its record before exiting; it has no other chance.

### 4.2 The recovery notice

On start-up, and whenever a record is added while a window is open, the frontend shows each **unacknowledged** record as a toast:

- **It stays until closed:** no expiry, not cleared on hover-out, not removed by a later toast. Closing it (X) acknowledges the record (srv writes `acknowledged_at_ms`), so it doesn't come back in another window or on the next launch.
- **It says what happened, why, and what was saved**, from the record: title from `kind` (§4.4), body `what` + `cause`, then "AgentMux saved:" and the `did` items, then "May be lost:" and `lost` if any.
- **Actions:** **Details** opens the incident in the backend popover (§4.3); **Copy report** copies the record with its evidence, for an issue.
- **One per incident**, newest first. More than three at once collapse into one: "AgentMux recovered 5 times since <time>" with Details.
- **In every window**: acknowledging in one closes it in all (the toast's id is the record's id; srv broadcasts the acknowledgement).
- **When no window is in front**, the same notice also goes out as an OS toast (the notify Router, a new kind `Recovered`, setting `notify:os:recovered`), since the app may have been restarted behind other windows.

### 4.3 Where it lives: a toast for the moment, the status bar for the record

The status bar's backend item is the right home for the **record**, not for the **notice**:

- A resource warning is a **state**: it is true for a while, and the dot shows it while it lasts. A recovery is an **event**: it is over by the time the user can look, and it needs to be read once. A yellow dot the user doesn't open says nothing about what happened.
- So: the **toast** carries the event to the user, once, until they close it (§4.2). The backend popover keeps the **history**: a "Recent incidents" section under `HealthNotices`, the last 20 records, newest first, each with when, what, why, what was saved, and Copy report.
- The dot gets no new colour for incidents. While an incident is unacknowledged, the uptime text carries a small marker ("● 2:14:05 ⓘ"). The marker goes when the notice is closed.
- After a restart, the uptime restarts from zero; its tooltip says "Restarted <time> after <kind>", from the newest record.

### 4.4 The incidents and their wording

| `kind` | Written by | Title | `did` (examples) |
|---|---|---|---|
| `renderer_crashed` | host, `crash_recovery.rs` | "A window crashed and was reloaded" | reloaded the window; agents kept running; nothing in the panes was lost |
| `renderer_oom` | host | "A window ran out of memory and was reloaded" | as above; the memory it was using when it stopped |
| `gpu_process_crashed` | host (detection: Phase C, §5) | "The graphics process crashed and was restarted" | windows redrawn; nothing lost |
| `window_closed_by_page` | host, `do_close` / the watchdog | "A web page closed the AgentMux window" | (with §6 this no longer happens; kept for any other path that removes the last window) |
| `quit_no_window` | host, `wrr` before its fast exit | "AgentMux quit because no window was left" | saved the layout; every agent's conversation is kept |
| `host_restarted` | launcher | "AgentMux restarted after it stopped unexpectedly" | restored your windows and panes; agents resumed |
| `host_restarted_safe_mode` | launcher (`--disable-gpu` step-down) | "AgentMux restarted without graphics acceleration" | as above, and why, and how to turn it back |
| `srv_restarted` | launcher, srv start-up | "AgentMux's background service restarted" | N agent turns were cut off; each resumes when you open it |
| `hang_restarted` | launcher | "AgentMux stopped responding and was restarted" | restored windows; a crash report was saved at <path> |
| `memory_give_up` | launcher | "AgentMux stopped: the computer was out of memory" | what it closed, what is kept |

Each `did` line is written only when that path actually did it.

### 4.5 First, make toasts show

None of this works while toasts are dev-only (§3.1), so the first step is the toast system itself:

1. Mount `<NotificationBubbles />` in every build.
2. Before that ships, go through the 23 call sites once: every toast that will now appear in release must be worth showing to a user (wording, expiry, type). The background-audit toast ("Ran while you were away") gets the same sticky handling as a recovery notice, since it reports a past event too.
3. The X **removes** a toast (and calls an `onClose` the sender can use: a recovery notice acknowledges its record). No hidden toasts left in the list.
4. A cap: at most five shown, the rest as "+N more", oldest dropped first, sticky toasts last.
5. `persistent: true` means no expiry and a visible "stays until closed" cue; sticky toasts aren't closed by hover-out or by other toasts.

## 5. Phases

- **A: toasts that show** (§4.5). Small and independent; most of the value of every other toast depends on it.
- **B: the record and the notice for the host's own cases** (§4.1, §4.2, §4.4: `renderer_crashed`, `renderer_oom`, `quit_no_window`, `window_closed_by_page`), the "Recent incidents" section and the uptime marker (§4.3).
- **C: the launcher's and srv's cases** (`host_restarted`, `host_restarted_safe_mode`, `srv_restarted`, `hang_restarted`, `memory_give_up`), the OS toast when no window is in front, and GPU-process crash detection (Chromium restarts it without a callback CEF exposes; candidates: the browser process's child-process exit, a GPU info change, the compositor losing its surface; to establish).

## 6. The incident's cause (fixed in #5024)

#5024 (merged the same day) holds back a browser pane's own `window.close()` in `do_close` on Windows (`browser_pane/page_close.rs`: CEF sends no `WM_CLOSE` to the window) and the frontend then closes that pane (`browser-model.ts`). Linux and macOS, where panes are CEF Views, keep CEF's default close, untested there (`docs/retro/RETRO_BROWSER_PANE_WINDOW_CLOSE_QUITS_APP_2026_10_16.md`). An alternative that replaced the pane in place with a "the page closed itself" page (#5026) was closed as a duplicate. The recovery record (§4.1) still applies to any path that removes the last window.

Two smaller findings from the same logs:
- The host logs every draggable-region update, about once a second with ~140 regions each: 136 MB of a 0.59.28 host log and 212 MB of a 0.59.26 one in a day. Log a change in the count, not every update.
- Each of the three windows' "[PRODUCTION BUILD] Loading from dist/frontend" lines at every pool refill is noise of the same kind.

## 7. Tests

- **Toasts:** a release-mode render shows a toast; X removes it and calls `onClose`; a sticky toast outlives its neighbours and hover-out; the cap collapses the rest into "+N more".
- **Records:** each Phase B path writes one record with the right `kind` and only the `did` lines it performed; a fast exit writes before exiting; the log keeps the last 200.
- **Notice:** an unacknowledged record shows on start-up in every window; closing it in one acknowledges it everywhere and it doesn't return on the next launch; more than three collapse into one.
- **Status bar:** the popover lists recent incidents; the uptime marker shows while one is unacknowledged and goes when it's closed.
- **§6:** a pane page's `window.close()` leaves the main window and the other panes open.

## 8. Open questions

1. Should agents hear about a recovery too (an `[AgentMux]` note, "AgentMux restarted at 13:42; your last turn was cut off"), or is the existing interrupted-session handling enough?
2. Retention: 200 records, or by age (30 days)?
3. Should the record be shared across channels (a `task dev` build crashing is that build's business), or per instance as proposed?
