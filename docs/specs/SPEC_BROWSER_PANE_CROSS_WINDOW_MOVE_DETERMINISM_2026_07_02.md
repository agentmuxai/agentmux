# Spec: Deterministic cross-window browser-pane moves (kill the create-vs-create race)

**Date:** 2026-07-02
**Author:** Agent2
**Status:** Draft
**Area:** `agentmux-cef` browser-pane lifecycle (host reducer + pane manager) ↔ `agentmux-srv` layout saga
**Depends on / follows:** PR #1890 (window-aware create + close). This spec addresses the *residual* race that #1890 does not close.

---

## 0. TL;DR

PR #1890 made browser-pane **create** and **close** window-aware, fixing the simple tear-off black screen (one source window → one destination window). But a **multi-window dock** — docking a *floating* browser into a *second top-level window* — still intermittently goes black.

Root cause: during the move, **both** windows briefly own the block in their local layout and each fires a `browser_pane_create`. The host reacts to *whichever create arrives* by moving the pane into that window (close-old + recreate-new). With two live claimants the moves **ping-pong**, and an in-flight page load gets `ERR_ABORTED` → black. Whether it renders or blacks out depends purely on arrival timing — it's a race.

The fix: make the move **deterministic** by gating it on the **authoritative destination** the srv layout saga (`RedockFloating` / `TearOffBlock`) commits, instead of reacting to non-authoritative frontend creates.

---

## 1. Background: what #1890 already fixed

`BrowserPaneEntry` now carries `window_label`. Two window-aware behaviors were added:

- **Create (`reducer/panes.rs`):** a `Live` entry whose window ≠ the create's target returns `RegisterResult::AlreadyLiveElsewhere`; the host closes the old pane and replays the create as `Fresh` in the requested window (reusing the `Closing` defer/replay machinery).
- **Close (`ipc.rs` + `browser-view.tsx`):** `browser_pane_close` carries the caller's `window_label`; the host ignores the close if the pane's current window no longer matches (a stale close from a window that no longer owns the pane).

This deterministically fixes **one-source → one-destination** moves.

---

## 2. The residual race (evidence)

Repro: window A has a **floating** browser pane. Open window B (a second top-level window, promoted from the pane pool). Drag-dock the floating pane into B.

Host-log trace (block `dd779390`, condensed):

```
seq61  close-ignored-stale-window  from=window-pool-384d59  current=floating-f3fd8   ← #1890 part-2 ignored a stale close
seq62  load-end (floating)
seq63  close  (view-unmount)
seq64-65  create-request → create-parent  requested=window-pool-384d59   ← window B claims the block
seq66  create-cross-window-move  old=-14  requested=floating-f3fd8         ← ...floating re-claims it
seq67  close
seq68-69  create-request → create-parent  requested=floating-f3fd8
seq70  load-error  ERR_ABORTED(-3)   ← in-flight load killed by the bounce
seq71  load-end
```

Both `window-pool-384d59` (window B) and `floating-f3fd8` (window A's floater) issue creates. Each cross-window create triggers a close-old + recreate-new, so the pane **oscillates** between the two windows. One page load is aborted mid-flight. The pane ends up resolved in the *wrong* window (`floating`), so window B — where the user dropped it — is black. **Retrying often works** because a different interleaving settles cleanly: it is timing-dependent.

### Why #1890's window-aware close doesn't cover this
Part 2 suppresses the *frontend unmount close* from the losing window. But the ping-pong here is **create-vs-create**, not create-vs-close: two windows both actively *create*. Part 1 (move-on-cross-window-create) is fighting itself.

---

## 3. Root cause

The host treats **every** `browser_pane_create` as authoritative intent to place the pane in that window. During a cross-window move there is a transient window where **two** frontends both believe they own the block (their local layout hasn't reconciled yet), so both create. The host has **no notion of which create is authoritative** — it just honors the latest, and the moves bounce.

The authoritative truth lives in the **srv layout reducer**: `RedockFloating` / `TearOffBlock` sagas commit the block to exactly one destination window. The host currently does not consult that when deciding whether to honor a cross-window create.

---

## 4. Goal

Make a cross-window move **deterministic and idempotent**: no matter how many competing `browser_pane_create` calls arrive from how many windows, the pane ends up in the **one window the layout saga committed it to**, with **no aborted loads** and **no bounce**.

---

## 5. Design

### 5.1 Establish the authoritative destination in the host

The srv sagas (`RedockFloating`, `TearOffBlock`) already run and are logged (`[dnd:svc] RedockFloating / TearOffBlock`). We need the host to learn, per `block_id`, the **committed target window** of the most recent move.

Options (in preference order):

- **D1 — Saga → host handoff (preferred).** When the saga commits a move, the srv emits an event carrying `{ block_id, target_window_label, move_seq }`. The host records `authoritative_window[block_id] = (target_window_label, move_seq)` in `HostState`. `move_seq` is a monotonic counter per block so later moves supersede earlier ones.
- **D2 — Reuse the existing pending-window / dnd plumbing.** The tear-off/redock already threads a target through `pending_window_creations` / the dnd complete path (`commands/drag.rs`, `commands/window.rs`). If the committed target window is already available there, thread it into the pane create call so the host doesn't need a new event.

### 5.2 Gate cross-window create/move on the authoritative target

In `handle_try_register_browser_pane_live` (and the `AlreadyLiveElsewhere` arm):

```
when a create arrives for block B targeting window W:
    let auth = authoritative_window[B]            // may be None if no move in flight
    if auth is Some(target, _) and W != target:
        // Non-authoritative create from a window that is NOT the committed
        // destination. Do NOT move. Ignore (or no-op re-nav in place).
        return AlreadyLive(existing)  // or a new "IgnoredNonAuthoritative" result
    else:
        // W == target (or no move in flight) → honor as today
        ... AlreadyLiveElsewhere / Fresh ...
```

Effect: only the create whose window matches the saga's committed target performs the move. The losing window's create is ignored, so there is **nothing to bounce**. The losing window's frontend will unmount shortly (its layout reconciles), and #1890's window-aware close already prevents that unmount from harming the winner.

### 5.3 Clear the authoritative record

`authoritative_window[block_id]` is set on saga commit and cleared when:
- the move completes (the pane is `Live` in the target window and has loaded), or
- a newer move for the same block supersedes it (`move_seq` increases), or
- the pane is fully closed.

Keep it a small bounded map; never let a stale entry outlive the move (mirror the `pending_browser_pane_creates` discipline — removed under the same lock that observes completion).

### 5.4 Abort-load hardening (defense in depth)

Even with gating, guard against a create that lands mid-load: when honoring a move, if the target pane is already `Live` and loading the same URL, prefer a **no-op** over close+recreate (don't abort a good load to redo it). This removes the `ERR_ABORTED` failure mode if any non-authoritative create slips through.

---

## 6. Acceptance criteria

- **AC1** Dock a floating browser into a second top-level window **20× in a row** → renders every time, zero black panes, zero `ERR_ABORTED` in the host log.
- **AC2** The host log shows **at most one** `create-cross-window-move` per dock (no oscillation), and any non-authoritative create is logged as ignored.
- **AC3** Simple tear-off (main ↔ floating) still works (no regression of #1890).
- **AC4** Rapid repeated tear-off/redock of the same pane never leaves it orphaned or in the wrong window.
- **AC5** No leaked `authoritative_window` entries after moves settle (assert map empties).

---

## 7. Test plan

- **Reducer unit tests:** competing creates for the same block from two windows with an authoritative target set → only the target-matching create moves; the other returns the ignore result. Superseding `move_seq` wins.
- **Loop/stress:** scripted tear-off↔dock N× (the AC1 repetition) with a log assertion for zero `ERR_ABORTED` / zero `create-cross-window-move` oscillation.
- **Manual:** the two-window dock repro from §2.

---

## 8. Risks & notes

- **Saga↔host coupling (D1)** adds a new cross-process signal; keep it additive and idempotent so an absent/late signal degrades to today's behavior (never worse than #1890).
- This is **browser-lifecycle-sensitive** (airspace / native HWND / pane pool). Land behind the existing analysis-doc discipline; review against the isolation invariants and the lifecycle spec.
- Prefer **D2** if the committed target is already reachable in the dnd complete path — it avoids new plumbing.

---

## 9. References

**Code**
- `agentmux-cef/src/reducer/panes.rs` — `handle_try_register_browser_pane_live`, `AlreadyLiveElsewhere`
- `agentmux-cef/src/browser_panes.rs` — `create()` (`AlreadyLiveElsewhere` arm), `close()`
- `agentmux-cef/src/ipc.rs` — `browser_pane_close` (window-aware)
- `agentmux-cef/src/state.rs` — `BrowserPaneEntry.window_label`, `browser_pane_window_label`
- `agentmux-cef/src/commands/drag.rs`, `commands/window.rs`, `commands/window_pool.rs` — tear-off / dnd / pool-promote target plumbing
- `agentmux-srv` layout saga — `RedockFloating`, `TearOffBlock` (`server::service` `[dnd:svc]`)

**Related analyses**
- `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md` — the create-side smoking gun (#1890 part 1)
- `docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md` — the `Closing` defer/replay machinery reused here
- `docs/analysis/ANALYSIS_FLOATING_PANE_REDOCK_SIZE_2026_06_23.md`

**PRs**
- #1890 — window-aware create + close (the prerequisite this spec builds on)
