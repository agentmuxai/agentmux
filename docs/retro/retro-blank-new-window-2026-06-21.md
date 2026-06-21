# Retro: "Open another window" yields a blank window (missing the 3 default panes)

**Date:** 2026-06-21
**Severity:** Medium (functional regression; new windows are unusable until the user manually adds panes)
**Area:** `agentmux-srv` window/workspace creation + frontend new-window bootstrap
**Status:** Fixed (branch `agentc/fix-blank-new-window`)
**Repro:** v0.47.0 (`553ff39b`). Status bar → "+ Open another window" → window opens **blank**; expected the default 3-pane layout (agent + sysinfo + swarm).

---

## 1. Expected vs actual

**Expected:** a new window opens with the default launch layout — three panes:

```
┌────────────────┬──────────────┐
│                │   sysinfo    │  ~20%
│     agent      ├──────────────┤
│    (tall)      │    swarm     │  ~80%
└────────────────┴──────────────┘
      50%              50%
```

**Actual:** the window opens with an empty workspace/tab — no panes.

---

## 2. The path "Open another window" actually takes

| Step | Code | Behaviour |
|------|------|-----------|
| 1. Button | `frontend/app/statusbar/InstancePanel.tsx:527` → `handleOpenNewWindow` (`:270`) | calls `getApi().openNewWindow()` |
| 2. Host | `agentmux-cef … open_new_window` → window pool | promotes a pre-spawned pool window via the **`pool:new-window`** event |
| 3. Pool | `frontend/app/init/pool.ts:33` | `pool:new-window` "removes `pool=1` but leaves `workspaceId` absent so `initHostNewWindow` creates a **fresh workspace**" |
| 4. FE bootstrap | `frontend/app-init.ts:311/323/337` | `WindowService.CreateWindow(null, "")` — empty `workspaceId` |
| 5. Backend | `agentmux-srv/src/server/service.rs:751` (`"CreateWindow"`) | empty ws → dispatch `Command::CreateWorkspace { name: "" }` (`:758`) + `CreateTab` |
| 6. Tab | `agentmux-srv/src/backend/wcore/tab.rs:41,45` | tab created with `rootnode: None`, `pendingbackendactions: None` |

Net: a workspace + an **empty tab with no layout and no blocks** → blank window.

The backend comment at `service.rs:749` states the intent outright:

> "Phase E.5.8 — CreateWindow migrated through the reducer … Layout setup for the
> new tab uses the apply_tab_created provisioning … **default rootnode = None matches
> wcore behaviour**."

---

## 3. Why it's blank: the default layout was welded to first launch

The default 3-pane layout is built in **exactly one production location**:
`agentmux-srv/src/backend/wcore/mod.rs::ensure_initial_data` (lines ~114-200), which
runs **only on first launch** (empty store: `if !clients.is_empty() { return Ok(false) }`).
It inserts the `agent`, `sysinfo`, `swarm` blocks and builds the `rootnode` inline.

Two facts confirm this is the *only* copy:
- The `three_pane_json` in `agentmux-srv/src/backend/obj.rs:643` is a **test fixture**,
  commented "matches wcore::mod.rs" — not a reusable builder.
- `git log -S 'sysinfo_block'` shows the layout was introduced by **#478**
  (`79555da6` — "feat(bootstrap): default 2-column launch layout (agent + sysinfo +
  swarm)"). `git show 79555da6` touched **only `wcore/mod.rs`** — i.e. it was
  implemented as a first-launch bootstrap special case, never as a shared
  "new-workspace defaults" primitive.

So no path other than first launch has ever asked for, or been able to produce, the
default layout from the backend.

---

## 4. How it broke (regression vector)

The defect is the interaction of three changes, none of which carried the default
layout into the general new-window path:

1. **#478 (`79555da6`)** put the default layout in `ensure_initial_data` only —
   first launch. A latent gap from day one.
2. **Reducer migration, Phase E.5.8 (`3a85b2b98`, 2026-04-30)** routed `CreateWindow`
   through the reducer and **explicitly set `rootnode = None`** for the fresh-workspace
   path ("matches wcore behaviour"). This is the line that makes a new window's tab
   empty.
3. **Window-pool new-window promotion** (`#1595` macOS/Linux, `#1612` Windows,
   `2e117808`) made "Open another window" / Cmd+N **always** flow through
   `pool:new-window → initHostNewWindow → CreateWindow(null,"")` — i.e. step 5/6 above.

The dead giveaway that defaults were *meant* to be requestable: the frontend
`WorkspaceService.CreateWorkspace(name, icon, color, **applyDefaults**)`
(`frontend/app/store/services.ts:146`) has carried an `applyDefaults` argument since
the **initial commit** (`4be0e8d4`, v0.31.20) — but the backend IPC
`Command::CreateWorkspace { name }` (`agentmux-common/src/ipc.rs:257`) only ever
carried `name`. `applyDefaults` (and `icon`/`color`, since removed as "unused" in
`c7cb07c2`) are **silently dropped** at the IPC boundary. The knob to seed defaults on
a fresh workspace exists in the UI layer and connects to nothing.

So: the layout was a first-launch special case; the `applyDefaults` wire that would
have generalized it was never connected; and the reducer + pool reworks made every new
window take the path that produces `rootnode = None`. The 3 panes simply never travel
with "Open another window."

---

## 5. Evidence summary

- **Repro:** confirmed live on v0.47.0 — new window opens blank.
- **Single source of the layout:** `wcore/mod.rs::ensure_initial_data` (first launch
  only); `git show 79555da6` proves #478 added it there and nowhere else.
- **New-window path produces no layout:** `service.rs:749` ("default rootnode = None"),
  `wcore/tab.rs:41` (`rootnode: None`), no default-block insertion in `app-init.ts`
  `initHostNewWindow` (grep for `sysinfo`/`swarm` in frontend bootstrap → none).
- **Dead `applyDefaults` wire:** present in `services.ts:146`, absent from
  `ipc.rs:257 Command::CreateWorkspace`.

---

## 6. Fix (implemented)

1. **Extracted a shared primitive** `wcore::seed_default_layout(store, tab_id)` from
   `ensure_initial_data` — it creates the agent/sysinfo/swarm blocks and builds the
   `rootnode`/`leaforder` (the exact tree previously inlined in `mod.rs`).
   `ensure_initial_data` now calls it, so first launch and new windows share one
   definition of "the default layout."
2. **Seeded it on the new-window path:** in `server::service`'s `CreateWindow`
   handler, the **empty-workspace branch** (after `CreateWorkspace` + `CreateTab`)
   calls `seed_default_layout` on the freshly-created tab. The `else` branch
   (existing workspace = tear-off) is untouched, so tear-off still reattaches its
   populated workspace as-is. The seed is **non-fatal**: a failure logs a warning and
   leaves an empty tab (the prior behaviour) rather than failing window creation.
3. **Chose the targeted seed over wiring `apply_defaults` through the IPC command.**
   Threading a new field through `Command::CreateWorkspace` + the reducer + the
   persist subscriber is a larger, riskier surface; seeding directly in the one
   user-facing new-window path (mirroring how `ensure_initial_data` seeds directly into
   the store) is smaller and equivalent in effect. Wiring `apply_defaults` end-to-end
   (and removing the dead frontend param) remains a reasonable follow-up cleanup.
4. **Regression test** `seed_default_layout_creates_three_pane_layout` (in
   `wcore/mod.rs`): a fresh tab starts blank (`rootnode == None`), and after
   `seed_default_layout` the tab has 3 blocks (`agent`/`sysinfo`/`swarm`) with a
   populated `rootnode` and a 3-entry `leaforder`. None existed before — the only
   3-pane assertion was the `obj.rs` shape *fixture*, which never ran against the
   real seeding path.

**Verification:** `cargo check -p agentmux-srv` clean; `cargo test -p agentmux-srv`
green incl. the new test.

---

## 7. Lessons

- **Defaults belong in a shared primitive, not a launch-time special case.** "First
  launch" and "new window" both need "a fresh workspace with the default layout"; they
  diverged because the layout was inlined into the bootstrap path.
- **A UI parameter that the IPC layer drops is a silent trap.** `applyDefaults` looked
  wired end-to-end from the frontend; it terminated at a `Command` that ignores it. A
  typed command whose fields mirror the service signature would have surfaced this.
- **Migrations that say "matches old behaviour" should name *which* old behaviour.**
  `rootnode = None` matched `create_window_full` (blank) but not `ensure_initial_data`
  (3-pane) — and new windows wanted the latter.
