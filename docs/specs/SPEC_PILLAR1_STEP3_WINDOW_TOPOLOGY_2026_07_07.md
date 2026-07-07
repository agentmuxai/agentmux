# Pillar 1 Step 3 — Persist Window Kind + Parent Linkage to srv

**Date:** 2026-07-07
**Type:** Sized implementation spec
**Status:** Ready to implement
**Builds on:** SPEC_864 (merged), SPEC_PILLAR1_STEP2 (merged) — this spec follows the exact same
shape as those: a small, host-only fact gets a durable srv counterpart, via a direct RPC, no reducer
machinery.
**Corrects:** `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §2.A/§6 step 3 — see that doc's
2026-07-07 addendum for why "audit completeness" was the wrong framing (there is nothing to audit;
the facts were never written anywhere durable).

---

## 0. TL;DR

`Window.kind` (`FullInstance`/`Subwindow`) and parent-window linkage (which window a Subwindow
belongs to) exist **only** in the launcher's in-memory `WindowMirror` map — a third process, with no
disk persistence at all. Killing the launcher (not just the host) loses this instantly; the
persisted `Window` row in srv (`agentmux-srv/src/backend/obj.rs`) has no field for either. This
spec adds both, writable via a direct RPC at window-creation time, following the exact
read-modify-write shape `SetWindowOpacity` already established. This is the prerequisite Step 4
(actual reproject) needs to know which windows to recreate and how — without it, a reproject can
recover *one* window's content perfectly but has no way to know a second window (or a subwindow)
ever existed, let alone its kind or its parent.

## 1. Current state (verified against source)

- **Durable today:** `Client.windowids: Vec<String>` (`agentmux-srv/src/backend/obj.rs:314-327`) —
  survives a full kill of launcher + host + srv (same data dir), since a crash never calls the
  explicit `CloseWindow` RPC that prunes it (`agentmux-srv/src/server/service/window.rs:394-396`).
  So "how many windows, which ids" is already recoverable.
- **NOT durable, anywhere:** the persisted `Window` struct (`obj.rs:333-358`) has fields
  `oid, version, workspaceid, isnew, pos, winsize, lastfocusts, opacity, meta` — no `kind`, no
  `parent`. `WindowKind` (`agentmux-common/src/ipc.rs:848-853`, `FullInstance`/`Subwindow`) and
  parent linkage (`parent_instance_id`) live only in:
  - The launcher's `WindowMirror` (`agentmux-launcher/src/state.rs:94-166`, `parent_label:
    Option<String>` at `:100`) — in-memory only, no serialization anywhere in that crate.
  - The host's own `WindowMeta` (`agentmux-cef/src/state.rs:77-82`) — also in-memory, rebuilt each
    host launch from the (in-memory) launcher's shadow projection.
  - The reducer command that creates a window (`HostCommand::EnqueuePendingWindowCreation`,
    `agentmux-cef/src/commands/window/creation.rs:395-398`) — fired once at creation time, never
    sent to srv.
- **Cold-launch behavior today:** `agentmux-cef/src/app.rs::on_context_initialized` creates exactly
  one native window unconditionally (implicitly "main"); the frontend inside it reads only
  `Client.windowids[0]` (`frontend/app-init.ts:317-339`). A second window or a subwindow is *never*
  automatically recreated — only ever by explicit user/agent action
  (`open_new_window`/`open_subwindow`, `agentmux-cef/src/commands/window/creation.rs:237-332`).

## 2. Target design

**New fields on `Window`** (additive, `#[serde(default)]`, mirrors `opacity`'s shape exactly —
`agentmux-srv/src/backend/obj.rs`):
- `kind: Option<String>` — `"full_instance"` | `"subwindow"`. `None` = unknown/legacy row (treat as
  `full_instance` on read, matching today's implicit default).
- `parent_window_id: Option<String>` — the parent `Window.oid`, set only for `subwindow`.

**New RPC**, same direct-store shape as `SetWindowOpacity` (`agentmux-srv/src/server/service/
window.rs:437-459` is the precedent to mirror, not the reducer): `SetWindowTopology(window_id, kind,
parent_window_id)`. Not reducer-routed, for the same reason opacity wasn't: the srv reducer's
`WindowRecord` tracks only `{window_id, workspace_id}` — kind/parent were never reducer state, so
there's no split-brain risk to guard against (confirmed pattern from SPEC_PILLAR1_STEP2 §2.A).

**Host write-through — the open question requiring verification at implementation time:** unlike
opacity (host-only state, host always has the value to write), `kind`/`parent_instance_id` are
currently known at creation time only in two places, and **the frontend does not currently know its
own window's kind at all** (confirmed: `is_main_window` is a pure `label == "main"` string check,
`agentmux-cef/src/commands/window/meta.rs:64-67`; grepped `frontend/app-init.ts` for
`kind`/`parent_instance_id` — zero hits). Two candidate write-through points, to be resolved during
implementation, not guessed here:
1. **Host-side, at creation**: `open_new_window`/`open_subwindow`
   (`agentmux-cef/src/commands/window/creation.rs:237-332`) already receive `kind`/
   `parent_instance_id` as call parameters — thread a write-through call there, after the backend
   `Window` row exists (i.e. after the frontend's `CreateWindow` RPC resolves, which the host isn't
   synchronously waiting on today — needs a signal back, e.g. piggyback on
   `register_backend_window`/`report_backend_window_id_registered`, which already fires once the
   frontend confirms a `window_id` for a label).
2. **Frontend-side**: thread `kind` through the existing `?windowLabel=`/tear-off URL param
   mechanism (subwindows already get a distinguishing URL context per
   `commands/window/creation.rs`'s call sites) and call the new RPC directly after
   `WindowService.CreateWindow` resolves in `initHostNewWindow()` (`frontend/app-init.ts:421-493`).

**Resolved (2026-07-07, Phase 2 implementation): Candidate 1.** `register_backend_window`
(`agentmux-cef/src/commands/window/meta.rs`) is exactly the right hook — it fires once per window
with that window's concrete srv `window_id`, and the host already has `WindowMeta{kind,
parent_instance_id}` for the label at that point (populated at creation time). One nuance found
during implementation: `WindowMeta.parent_instance_id` is a window **label**, not a srv id — it must
be resolved to the parent's `window_id` via `AppState::backend_window_id(parent_label)` before
writing srv's `parent_window_id` field (the same label→id lookup the opacity/floating-placement
write-throughs already use). If the parent hasn't registered its own `window_id` yet (a narrow
creation-order race — in practice a subwindow's parent is always already open, so this is
theoretical), the write-through is skipped for that call rather than persisting a wrong value.

**Reproject read-back (Step 4's dependency, not this spec's job):** once persisted, a future
reproject pass can `GetWindow` each id from `Client.windowids[1..]`, read `kind`/`parent_window_id`,
and know which native-window-creation call to drive for each — this spec only makes that data exist
somewhere; consuming it is Step 4.

## 3. Phased plan

**Phase 1 — srv side. ✅ Done, merged (PR #2004).** Added `Window.kind`/`parent_window_id` fields +
`SetWindowTopology` RPC arm (direct store read-modify-write, mirroring `service/window.rs`'s
`SetWindowOpacity`). Validates `kind` against the known enum, rejects `subwindow` without a
`parent_window_id`, and (added during reagent review) rejects a `parent_window_id` set alongside
any kind other than `subwindow`, a dangling `parent_window_id` (doesn't reference a real window), or
a self-referential one. 10 unit tests, behavior-neutral at merge (nothing called it yet).

**Phase 2 — host/frontend wiring. ✅ Done.** Wired via `register_backend_window` (Candidate 1
above). **Live-verified** on an isolated instance: `main` → `kind: "full_instance"`, no
`parent_window_id`; a second full window opened via `openNewWindow()` → `kind: "full_instance"`, no
`parent_window_id`; a subwindow opened via `open_subwindow {parent_instance_id: "main"}` →
`kind: "subwindow"`, `parent_window_id` == main's own srv `oid` (correctly resolved from the label,
not the literal string `"main"`) — confirmed via direct `GetWindow` RPC calls against srv, not just
host-side logging.

Each phase independently shippable, matching every other Pillar 1 spec's phasing discipline.

## 4. Risks / honest caveats

- **The write-through timing gap is real, not cosmetic.** If the host process crashes in the
  narrow window between a new window's backend `Window` row existing and the topology write-through
  landing, that window reprojects as `kind: None` (defaults to `full_instance`) on the next
  restart — parent linkage for a subwindow would be lost for that one window. Acceptable (same
  class of gap SPEC_PILLAR1_STEP2 already accepted for opacity/placement — a narrow, bounded
  data-loss window on an already-rare crash path, not a correctness bug), but should be stated
  explicitly rather than silently accepted.
- **This spec does not change cold-launch or crash-restart behavior at all.** Even after this
  lands, only window #1 is ever automatically recreated — see the parent design doc's 2026-07-07
  correction. This is purely making the data exist; Step 4 is a separate, larger, not-yet-scoped
  piece of work.

## 5. Explicitly out of scope

- Actually consuming `kind`/`parent_window_id` to recreate windows on reproject — Step 4.
- Any change to `open_new_window`/`open_subwindow`'s existing behavior for a live, non-crashed
  session — this is additive persistence only.

## 6. Definition of done

1. ✅ `Window.kind`/`parent_window_id` persist through `SetWindowTopology`, unit-tested (10 tests).
2. ✅ Opening a subwindow live-verifiably persists `kind: "subwindow"` + the correct parent id to
   srv (checked via `GetWindow`, same isolated-instance methodology as every other Pillar 1 spec
   this session).
3. ✅ Opening a second full window live-verifiably persists `kind: "full_instance"`,
   `parent_window_id: null`.

## 7. Sources

- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` (parent design doc, corrected
  2026-07-07).
- Code read for this spec (via a research pass, 2026-07-07): `agentmux-cef/src/app.rs`
  (`on_context_initialized`), `agentmux-cef/src/commands/window/creation.rs:237-419`,
  `agentmux-cef/src/commands/window/meta.rs:64-67`, `agentmux-cef/src/state.rs:77-82`,
  `agentmux-launcher/src/state.rs:94-297`, `agentmux-srv/src/backend/obj.rs:314-358`,
  `agentmux-srv/src/server/service/window.rs:437-459` (the `SetWindowOpacity` precedent),
  `frontend/app-init.ts:317-493`, `frontend/app/store/services.ts:108-109`.
