# Pillar 1 Step 2 — Persist the Two Host-Only Topology Facts to srv

**Date:** 2026-07-06
**Type:** Sized implementation spec (the deliverable `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`
called for as its own next step)
**Status:** Ready to implement
**Builds on:** #864 (layout single writer, merged 2026-07-06 — the hard prerequisite for this step)
**Resolves:** Step 2 of the 6-step Pillar 1 sequence in `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §6

---

## 0. TL;DR

Two pieces of "logical topology" (Q1 Category A in the design doc) currently live **only in the host's
in-memory reducer state**, with zero srv-side counterpart: per-window opacity, and floating-pane
placement (`last_known_normal_rect`). Verified by direct code read (not the design doc's prose) —
see §1.

**Correction to the design doc's framing:** it assumed floating-pane placement would write through to
the existing `Window.pos`/`Window.winsize` fields via the existing (currently dead) `SetWindowPosAndSize`
RPC. **That assumption is wrong.** Floating panes never call `WindowService.CreateWindow` and never
register a `backend_window_id` — confirmed by reading `agentmux-cef/src/commands/floating_pane.rs:129-130`,
where the "window_id" used to build the floating window's label is a **locally-generated UUID with no
srv registration at all**, not a real srv `Window.oid`. Floating panes have **no srv `Window` row to
write to.** This spec's design routes floating-pane placement through the **block's** `meta` map
instead (the floating pane's content block is the one srv entity that legitimately outlives the
native window and already gets tracked through tear-off/redock).

Per-window opacity, by contrast, applies to **real windows** (main + secondary — the ones that *do*
get a `backend_window_id`), so it can use the `Window` object directly.

**Recommended sequencing:** two independently-shippable slices, mirroring #864's phase structure:
- **Slice A — per-window opacity** (smaller, reuses `Window`, single existing host trigger point).
- **Slice B — floating-pane placement** (needs a new `Block.meta` write-through path; the drag/resize
  continuous-tracking half is explicitly deferred — see §5).

---

## 1. Current state (verified against source, not assumed)

### 1.A Per-window opacity — 100% host-local

- `HostState.window_opacities: HashMap<String, f32>` — `agentmux-cef/src/reducer/mod.rs:130`
  (keyed by window **label**, e.g. `"main"`, `"window-pool-<uuid>"`).
- Mutated only by `HostCommand::SetWindowOpacity { label, opacity }` (`mod.rs:432`) →
  `handle_set_window_opacity` (`mod.rs:1165-1178`, clamps `[0,1]`).
- IPC entry points: `set_window_opacity` / `get_window_opacity` in
  `agentmux-cef/src/commands/window/transparency.rs:135,228`.
- Win32 side-effect: `apply_window_opacity`/`remove_window_opacity` (`transparency.rs:99,119`) via
  `WS_EX_LAYERED` + `SetLayeredWindowAttributes`, using the `window_hwnds` HWND cache
  (`state.rs:860-866`). macOS/Linux equivalents exist in `agentmux-cef/src/ui_tasks/window.rs`
  (NSWindow `alphaValue`, X11 `_NET_WM_WINDOW_OPACITY`).
- **Read-back on window init reads host memory only** (`transparency.rs:228-244`) — a crashed/restarted
  host has no way to recover a window's last opacity from anywhere.
- Srv side: `grep -i opacity agentmux-srv/src` finds only the **global default**
  (`window:opacity` / `window:magnifiedblockopacity` in `wconfig/types.rs:140-141,164-165`) — no
  per-window value anywhere.

### 1.B Floating-pane placement — 100% host-local, AND no srv entity to attach to

- `HostState.pane_window_states: HashMap<String, PaneWindowState>` — `agentmux-cef/src/reducer/mod.rs:142`,
  keyed by floating-window label (`floating-<uuid>`, where the UUID is generated client-side and
  **never sent to srv** — `commands/floating_pane.rs:129-130`).
- `PaneWindowState { placement: WindowPlacement, last_known_normal_rect: Option<PaneRect> }` —
  `state.rs:202-213`. `WindowPlacement` = `Normal | Maximized | Minimized` (`state.rs:189-196`).
  `PaneRect` = `{left, top, right, bottom}` physical pixels (`state.rs:176-182`).
- Only mutation path: `HostCommand::ToggleFloatingMaximize { label, current_rect }` →
  `handle_toggle_floating_maximize` (`agentmux-cef/src/reducer/pane_window.rs:56-104`) — a **button-click**
  event, not continuous drag/resize tracking. Un-maximize pops `last_known_normal_rect` back out as the
  restore target.
- Actual rect capture: `agentmux-cef/src/commands/window/chrome.rs:96-217` `toggle_floating_maximize` —
  `GetWindowRect` **at click time only** (Windows), applied via `SetWindowPos`.
- Eviction on close: `HostCommand::EvictFloatingPaneWindowState` → `pane_window.rs:112-119`, fired from
  `on_before_close`.
- **Deferred, not built:** `ReportNormalRect` (debounced `WM_WINDOWPOSCHANGED` tracking for raw
  drag/resize, not just the maximize button) and `ReportOSPlacementChange` (Win+Down / system-menu
  placement) are documented as future work in `pane_window.rs:38-40` and
  `docs/specs/SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md:89-98` — **no such `HostCommand` variant
  exists in the codebase today.** Out of scope for this spec (§5).
- **No srv `Window` row exists for a floating pane at all** — confirmed: `open_floating_pane_window`
  (`commands/floating_pane.rs:100+`) never calls `WindowService.CreateWindow` or
  `report_backend_window_id_registered`. The floating pane's only srv-side identity is the **block**
  it displays (created via the existing `CreateBlock` + `tear_off_block` saga path,
  `agentmux-srv/src/server/app_api/pane.rs`).

### 1.C What already exists and is reusable

- `Window.pos: Point` / `Window.winsize: WinSize` (`agentmux-srv/src/backend/obj.rs:341,343`) — typed
  fields, currently **dead** (no reader or writer anywhere in the host or frontend). Reusable for
  Slice A's sibling concern (real windows) but NOT for floating panes (§1.B).
- `SetWindowPosAndSize` RPC (`agentmux-srv/src/server/service/window.rs:437-459`) — existing
  read-modify-write-via-wstore pattern; currently unused by anyone. A precedent for the write-through
  shape, not directly reusable for floating panes (which have no `Window` row to target) but *is*
  directly reusable if a future pass decides to also start writing real-window position/size (out of
  scope here — this spec covers opacity + floating-pane placement only, per the design doc's Q1 list).
- `Block.meta: MetaMapType` (`agentmux-srv/src/backend/obj.rs:463`) — the generic per-block metadata
  map already used for `"view"`, `"tab:color"`, etc. The natural target for floating-pane placement.
- `state.backend_window_id(label)` (`agentmux-cef/src/state.rs:1395-1396`) — host-side label→srv-`Window.oid`
  lookup, fed by `Event::BackendWindowIdRegistered`. Populated for **real windows** (confirmed via
  `register_backend_window`, `agentmux-cef/src/commands/window/meta.rs:182-197`, called generically
  by the frontend's window-bootstrap path). **Not populated for floating panes** (§1.B) — do not use
  this lookup for Slice B.

---

## 2. Target design

### 2.A Slice A — per-window opacity

**New field (agentmux-srv, additive — no migration risk, `#[serde(default)]`):**
- `Window.opacity: Option<f32>` in `agentmux-srv/src/backend/obj.rs` (mirrors the existing `pos`/`winsize`
  shape). `None` = fully opaque / unset, matching today's default behavior.

**Correction from an earlier draft of this spec:** the srv reducer's `WindowRecord`
(`agentmux-srv/src/state.rs:132-135`) tracks only `{window_id, workspace_id}` — position, size, and
now opacity are **not reducer state at all**, so there's no split-brain risk the way #864's layout
tree had two writers. The existing `SetWindowPosAndSize` RPC
(`agentmux-srv/src/server/service/window.rs:437-459`) reflects this correctly: it's a **direct
`store.must_get`/`store.update` RPC, not routed through `dispatch_to_reducer`.** `SetWindowOpacity`
should be a sibling RPC in the same handler, same shape — not a new reducer `Command`/`Event` pair.
Inventing reducer machinery for state the reducer was never meant to hold would be net-new complexity
with no coherence benefit.

**Host write-through (the actual new wiring):**
- `transparency.rs::set_window_opacity` / `remove_window_opacity`: after the existing Win32 side-effect,
  resolve `state.backend_window_id(label)`; if `Some(window_id)`, fire-and-forget an RPC call to the new
  `window.SetWindowOpacity` method (mirroring how `report_backend_window_id_registered` is a
  best-effort, non-blocking call today — a failed write-through must never block the opacity change
  the user is actively performing).
- **Read-back on window init:** after the existing host-memory lookup in `get_window_opacity`
  (`transparency.rs:228-244`) misses (fresh process, no in-memory entry — this is the crash-recovery
  case), fall back to reading `Window.opacity` from srv via `GetWindow`. This is the actual Pillar-1
  payoff: a crashed-and-restarted host recovers the opacity a cold-start-only host would have lost.

### 2.B Slice B — floating-pane placement

**New meta key on the floating pane's Block** (not a new typed field — `Block.meta` is exactly the
"per-entity loosely-typed sidecar" this fits, and matches the codebase's own forward-looking doc
comment naming it `pane:floating_normal_rect`):
- `block.meta["pane:floating_normal_rect"] = { "left": i64, "top": i64, "right": i64, "bottom": i64 }`
  (mirrors `PaneRect`'s shape exactly, JSON-encoded via the existing generic meta-patch mechanism —
  `agentmux-srv/src/persist_subscriber.rs`'s `apply_workspace_meta_updated`-style merge, or the
  existing `UpdateObjectMeta` RPC if it already supports block metadata; verify at implementation time).
- `block.meta["pane:floating_placement"] = "normal" | "maximized" | "minimized"` (mirrors
  `WindowPlacement`).

**Why the block, not a new `FloatingPaneState` WaveObj type:** the block already has a stable srv
identity that survives exactly the events that matter — tear-off (new block created), redock (block
moves tabs), and delete (block removed, meta goes with it). Inventing a parallel `FloatingWindowState`
object keyed by the host's ephemeral label would need its own lifecycle-matching logic for zero
benefit over reusing the block's.

**Host write-through — block_id resolution, verified (no longer an open question):**
- `PaneWindowState` (`agentmux-cef/src/state.rs:207-213`) has no `block_id` field — only `placement`
  and `last_known_normal_rect`. Confirmed no reverse label→block_id registry exists anywhere in
  `agentmux-cef` either; the only block-keyed map (`HostState.browser_panes`) goes block_id→label, not
  the other way, and resolving label→block_id from it would require an unenforced 1:1 linear scan.
- **The caller already has `block_id` in scope and doesn't need a host-side lookup at all.**
  `toggle_floating_maximize` (`chrome.rs:96-104`) is invoked from `FloatingMaximizeButton`
  (`frontend/app/block/blockframe.tsx:201-211`), itself rendered from `EndIcons`
  (`blockframe.tsx:282`) — which already holds `props.nodeModel.blockId` in scope two lines earlier
  (used for the mic button, `blockframe.tsx:255`). **Design change from the original draft:** thread
  `block_id` through as a new IPC arg (`invokeCommand("toggle_floating_maximize", { label, block_id })`)
  rather than having `chrome.rs` resolve it host-side. This is strictly simpler than adding a registry
  and avoids the 1:1-assumption fragility a lookup table would carry.
- With `block_id` supplied by the caller, `chrome.rs::toggle_floating_maximize` fires
  `ObjectService.UpdateObjectMeta` (or the block-metadata equivalent — confirm exact RPC name at Phase 3)
  with the two keys above, after capturing/restoring `current_rect`/`last_known_normal_rect`.
- **Creation-time resolution also verified:** `OpenFloatingPaneArgs.pane_id` (`floating_pane.rs:38`) is
  supplied by the frontend when the floating window is first created — likely already the block_id
  under a different field name, but this equivalence needs a one-time confirmation against the
  block/pane ID scheme (`agentmux-srv/src/server/app_api/pane.rs`) before Phase 4, not assumed. If
  `pane_id` ≠ `block_id`, `OpenFloatingPaneArgs` needs a new field for the read-back path.
- **Read-back on floating-pane re-open:** when `open_floating_pane_window` creates the native window
  for a block, read `block.meta["pane:floating_normal_rect"]`/`["pane:floating_placement"]` (if present)
  to restore geometry instead of falling back to the caller-supplied default width/height. This is
  the reproject payoff for floating panes specifically — a crash-restarted host puts floating panes
  back where they were, not back at a default size.

---

## 3. Phased plan

**Phase 1 (Slice A) — opacity, srv side.** Add `Window.opacity` and a `SetWindowOpacity` RPC arm
alongside `SetWindowPosAndSize` in `service/window.rs` (same direct store read-modify-write shape,
not reducer-routed — see the correction in §2.A). Unit tests: round-trip persists, unknown window
errors, `None`/clear works. Behavior-neutral — nothing calls it yet.

**Phase 2 (Slice A) — opacity, host wiring.** `transparency.rs` write-through on set/remove; read-back
fallback on init. **App-running verification required** (per the design doc's Q2/Q3 caution for
anything touching host↔srv write-through) — set an opacity, kill the host process (not graceful quit),
relaunch, confirm the window restores the opacity from srv rather than defaulting to opaque.

**Phase 3 (Slice B) — floating-pane placement, srv side. Done — no new code needed.** Confirmed
`UpdateObjectMeta` (`agentmux-srv/src/server/service/object.rs:291-389`) already routes `OTYPE_BLOCK`
through `Command::UpdateBlockMeta` → the reducer → `apply_block_meta_updated`/`merge_meta_patch`
(`persist_subscriber.rs:1013-1025,1168+`), a generic shallow merge over arbitrary `block.meta` keys
(including `null`-to-delete section-clear semantics). This already fully supports writing
`pane:floating_normal_rect`/`pane:floating_placement` — no srv-side PR needed for this phase.

**Phase 4 (Slice B) — floating-pane placement, host wiring. Write-through done; read-back deferred
(see below).**
- `chrome.rs::toggle_floating_maximize` now accepts an optional `block_id` in its IPC args (threaded
  through from `FloatingMaximizeButton` in `blockframe.tsx`, which already has `nodeModel.blockId` in
  scope — see the resolved §2.B design). After the reducer dispatch and Win32 geometry side-effect, it
  fires `client::backend_update_block_meta` (fire-and-forget, mirrors `backend_set_window_opacity`'s
  shape, no debounce needed since this is a button click, not a continuous drag) with
  `pane:floating_placement` always, and `pane:floating_normal_rect` when a rect is available (the
  pre-toggle `current_rect` on maximize, the reducer-returned `restore_rect` on restore).
- **`open_floating_pane_window` read-back on creation — deliberately NOT implemented in this phase.**
  Verified via `frontend/app/workspace/floating-pane-workspace.tsx:9-11`'s own doc comment: "The
  floating window's backend state is a normal workspace+tab+block (created by `TearOffBlock` in the
  source window)" — i.e. `OpenFloatingPaneArgs.pane_id` (`floating_pane.rs:38`) IS the block_id, and a
  *new* block is created by `TearOffBlock` on every single tear-off. That means
  `block.meta["pane:floating_normal_rect"]` can never be present the first time
  `open_floating_pane_window` runs for a given block — there is no call site today where a floater is
  recreated for a block that was PREVIOUSLY floating (redock removes the floating block entirely; a
  fresh tear-off always makes a fresh block). Wiring a read-back now would be dead code with no live
  trigger, not a verifiable feature — it only becomes meaningful once crash-reproject (Step 4 of the
  Pillar 1 sequence, explicitly out of scope per §5) actually recreates floating windows for
  surviving blocks after a restart. Revisit this read-back as part of Step 4, not here.

Each phase independently shippable + testable, mirroring #864's approach. Do not gate Phase 1/3 on
Phase 2/4 landing — the srv-side plumbing is valid and testable on its own.

---

## 4. Risks / honest caveats

- **Fire-and-forget write-through must not introduce a new UI-thread stall.** Both write-through call
  sites (`transparency.rs`, `chrome.rs`) are in the hot path of a user dragging/clicking a window
  control — an RPC round-trip must be async/non-blocking, matching the existing
  `report_backend_window_id_registered` pattern (send-and-forget over the launcher-ipc channel, not a
  synchronous wait).
- **Floating-pane block-id resolution — resolved, see §2.B.** No label→block_id registry exists
  host-side; the fix is threading `block_id` through as a new IPC arg from the frontend caller (which
  already has it), not a host-side lookup. One remaining sub-item before Phase 4: confirm
  `OpenFloatingPaneArgs.pane_id` (`floating_pane.rs:38`) is actually the block_id (or add a field if not).
- **This spec does not implement `ReportNormalRect`/`ReportOSPlacementChange`** (continuous drag/resize
  tracking). Only the existing maximize-button trigger gets write-through. A floating pane resized by
  dragging its edge (not via the maximize button) will NOT have its new size persisted until/unless
  those deferred commands are built — a known, bounded gap, not a regression from today's behavior
  (today persists nothing at all).
- **Opacity's `Window.opacity` field needs a decision on window-vs-workspace scope** — confirm at
  implementation time that `Window.oid` is the right granularity (vs. per-workspace or per-tab) by
  checking what `HostState.window_opacities`' keys actually resolve to in practice (labels for
  "main"/pool-promoted windows, which map 1:1 to `Window.oid` via `backend_window_id` — expected to be
  fine, but confirm no window ever legitimately needs per-tab opacity).

---

## 5. Explicitly out of scope

- Continuous drag/resize tracking for floating panes (`ReportNormalRect`/`ReportOSPlacementChange`) —
  a separate, already-scoped follow-up per the existing doc comments in `pane_window.rs:38-40`.
- Persisting real-window (`main`/secondary) position/size via the existing-but-dead
  `SetWindowPosAndSize` — genuinely useful for reproject but not one of the two facts the design doc's
  Q1 table names; a natural Step 2.5 if reproject's window-set audit (Step 3) finds it's needed.
- Firing the actual crash-reproject restore path (Step 4 of the Pillar 1 sequence) — this spec only
  gets the *facts* into srv; consuming them on a cold/crash restart is later work.

---

## 6. Definition of done

1. ✅ `Window.opacity` persists through a real `SetWindowOpacity` RPC call, unit-tested (Phase 1, #1982).
2. ✅ Setting a window's opacity in a running app persists it to srv (Phase 2 — verified live via an
   isolated instance: `window.api.setWindowOpacity('main', 0.7)` → `GetWindow` showed
   `opacity: 0.699999988079071` in srv).
3. ✅ Killing and relaunching the host restores the last-set opacity for windows that had one (Phase 2 —
   verified live: killed the host process, relaunched against the same channel,
   `window.api.getWindowOpacity('main')` on the fresh process returned `0.7`, not the 1.0 default).
   Also verified the clear path: `setWindowOpacity('main', 1.0)` → srv's `Window.opacity` field became
   fully absent (`None`, matching `WindowOpacityCleared`, not `Some(1.0)`).
4. ✅ Floating-pane placement (`pane:floating_placement`/`pane:floating_normal_rect`) persists to the
   block's `meta` map through the existing generic `UpdateObjectMeta` RPC (Phase 3 needed no new srv
   code — verified the generic shallow-merge mechanism already covers this) and the Phase 4 host
   write-through in `chrome.rs::toggle_floating_maximize`. Compiles clean (`cargo check -p agentmux-cef`,
   `cargo test -p agentmux-cef` — 159 passed), `tsc --noEmit` clean on the frontend change.
   **Verified live** on an isolated instance: created a real floating pane via `open_floating_pane_window`
   (block `596df4fc-…`, 500×400 at (300,300)), called `toggle_floating_maximize` with the new `block_id`
   arg — srv's `GetObject` showed `meta.pane:floating_placement:"maximized"` and
   `pane:floating_normal_rect:{left:300,top:300,right:800,bottom:700}` (`version: 2`). Toggled again
   (restore) — `placement` flipped to `"normal"`, rect unchanged, `version: 3`. Killed the outer
   `agentmux.exe` (not graceful quit) and relaunched against the same channel — `GetObject` on the same
   block still returned `placement:"normal"`, the same rect, `version: 3` unchanged — confirms the
   write-through survives a host crash (expected, since it bypasses host memory entirely, unlike
   opacity's `HostState.window_opacities` mirror).
5. ⬜ Read-back on floating-pane re-open — deliberately deferred, not a gap in this phase. See §3
   Phase 4: no call site exists today where `open_floating_pane_window` runs for a block that already
   has persisted placement meta (every tear-off creates a fresh block). Belongs to Step 4
   (crash-reproject), not this spec.
6. ✅ Both write-through paths (Phase 2) are `std::thread::spawn`/`spawn_blocking`, off the calling
   thread — no stall observed live.

---

## 7. Sources

- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §2.A, §6 (step 2).
- Code read for this spec: `agentmux-cef/src/reducer/mod.rs:125-142,432,452-457,771-793,1165-1178`,
  `agentmux-cef/src/reducer/pane_window.rs:35-119`, `agentmux-cef/src/reducer/browsers.rs:86-93`,
  `agentmux-cef/src/state.rs:176-213,860-866,1392-1396`,
  `agentmux-cef/src/commands/window/transparency.rs:99-244`,
  `agentmux-cef/src/commands/window/chrome.rs:96-217`,
  `agentmux-cef/src/commands/window/meta.rs:160-209`,
  `agentmux-cef/src/commands/floating_pane.rs:97-167`,
  `agentmux-srv/src/backend/obj.rs:333-348,454-467`,
  `agentmux-srv/src/server/service/window.rs:437-459`,
  `agentmux-srv/src/backend/wconfig/types.rs:140-141,164-165`,
  `frontend/app-init.ts:317-370` (confirms `CreateWindow` is a first-launch/main-window path, not a
  per-floating-pane trigger),
  `frontend/app/store/services.ts:123-125`.
