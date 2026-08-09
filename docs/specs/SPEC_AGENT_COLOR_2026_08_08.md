# SPEC: Per-agent color — assign at creation, backfill existing, show on the pane frame

**Date:** 2026-08-08
**Status:** Implemented (same-day PR)
**Author:** Agent3 (agent)
**Trigger:** User request (2026-08-05, re-confirmed 2026-08-08) — *"when new
agents are created, they should get a color … for now, generate a random
color for all the existing agents"* / *"can u see all agents with random
colors as a one-time script?"*
**History:** An earlier draft (`SPEC_AGENT_COLOR_2026_08_05.md`) was written
but never implemented, and was deleted untracked in the 2026-08-07 docs
cleanup. This spec replaces it with a narrower, shipping-first scope.

---

## 1. Goal

Every agent has a color. Existing agents get one via a one-time backfill;
new agents get one at creation. The color is immediately **visible** with
zero new rendering code, by reusing the pane frame's existing
`frame:bordercolor` block-meta support.

## 2. What exists to build on (verified against `main` @ `3fbe072bf`)

| Piece | Where | Why it matters |
|---|---|---|
| Per-agent KV content store | `Store::agent_content_get/set` (`backend/storage/content.rs`) — global cross-channel via the def-registry mirror; handles cross-channel agents (no local FK row) by routing to the registry directly | The established home for per-agent side-data (`ui:zoom` precedent, `SPEC_AGENT_ZOOM_PERSISTENCE_2026_06_22.md`). No schema change, no `AgentDefinition` struct change, no dual-write surface touched. |
| Block-meta seeding at open | `register_agent_open` (`server/app_api/agent_open.rs:297`) already seeds `term:zoom` from `agent_content("ui:zoom")` | The exact pattern for seeding `frame:bordercolor` from `ui:color`. |
| Pane frame color rendering | `blockframe.tsx:733` styles the block border from `meta["frame:bordercolor"]`; right-click pane-header swatch palette already writes this key | The display surface — colors show on every agent pane's frame with **no frontend changes**. The user can still override per pane via the existing palette. |
| Migration framework | `agentmux-srv/src/migrations/` (m0000–m0019), once-per-channel tracking in `db_migrations` | The codebase's mechanism for exactly-once backfills — the requested "one-time script". |
| 14-hue palette | `TAB_COLORS` (`frontend/app/tab/tab.tsx:20`) — also used for random tab colors (`tab-actions.ts:31`) | Color values to assign from. Duplicated as hexes in the Rust migration (a one-time list; drift is inconsequential since assigned colors are stored, not derived). |

## 3. Design

### 3.1 Storage

`agent_content` row: `content_type = "ui:color"`, `content = "#rrggbb"`.
Absent row = no color (pane frame falls back to default border, exactly as
today). No migration of the `AgentDefinition` schema.

### 3.2 Backfill (the one-time script) — migration `m0020_agent_color_backfill`

Channel scope, follows the `m0015` shape: open the channel store, iterate
`agent_def_list()`, and for every definition without an existing `ui:color`
row, write one. Color choice is a **deterministic hash of the agent id**
mapped onto the 14-hue palette — effectively random across agents, but
stable across re-runs/machines and dependency-free (no `rand` needed).
Skips defs that already have a color, so it's idempotent even beyond the
framework's once-per-channel tracking.

### 3.3 New agents — assign at creation

In the `createagent` handler (`server/agent_handlers/core.rs`), after
`agent_def_insert`, write `ui:color` with the same id-hash palette pick.
Fork/branch flows that create defs through other paths are left to the
backfill-on-open fallback (§3.4) rather than chasing every creation site.

### 3.4 Display seeding — `register_agent_open`

Next to the `ui:zoom` seeding block: read `agent_content("ui:color")`; if a
valid `#rrggbb` value exists, seed BOTH frame color keys into the new
block's meta — `blockframe.tsx` reads `frame:activebordercolor` for the
focused state and `frame:bordercolor` for the unfocused state, so seeding
only one would leave the color invisible half the time. Focused gets the
full-strength hex; unfocused gets a dimmed variant (RGB × 0.55, computed in
Rust) so the agent's color is visible either way while focus stays
distinguishable by brightness. Validation is a strict `#` + 6-hex-digit
shape check so a corrupt row can't inject arbitrary CSS. As a safety net
for defs created by paths §3.3 doesn't cover, if no color exists, assign
one here (write-through) before seeding — every agent that gets opened ends
up colored.

Discovered while implementing: nothing in the frontend writes the literal
`frame:bordercolor` KEY today — `SPEC_COLOR_PALETTE_EXPANSION_REUSE_2026_06_30.md`'s
pane-header swatch palette only shipped for tabs (`ColorSwatchPalette`'s
sole consumer is `tab.tsx`). **This check was incomplete** (reagent P1 on
the PR): it missed that `blockframe.tsx`'s border-color computation ALSO
reads `frame:hue`, written by the already-shipped, all-pane-types "Pane
Color" picker (`pane-color-menu.ts::setHue`, wired into every block's
header context menu — a *different*, already-shipped feature from the
06-30 spec, not the tab-only swatch grid). Before the fix below, once an
agent pane's `frame:activebordercolor` was seeded, it was checked AFTER
`frame:hue` in the focused-state style memo and unconditionally
overwrote it — so picking a hue on an agent pane had no visible effect,
permanently, for that block's lifetime (the seed is a one-time write, but
`blockData()` re-reads it on every render). Fixed in
`frontend/app/block/blockframe.tsx`'s focused-state branch by checking
`frame:activebordercolor` BEFORE `frame:hue` instead — the explicit user
choice now always wins when present; clearing the hue picker
(`frame:hue: null`) correctly falls through to the agent's default color
rather than to no color at all. The unfocused-state branch has no
`frame:hue`-derived value to conflict with (`hueToActiveBorder`'s hue
rendering only ever existed for the focused ring), so `frame:bordercolor`
needed no equivalent change there.

### 3.5 Architectural discovery: agent launch has TWO independent meta-building paths

Live verification against a running `task dev` build showed §3.4's seed
never took effect for the everyday "click **Continue** in the agent
picker" flow, despite `register_agent_open` visibly working (confirmed via
the migration, which shares its code) and the written block ending up with
every OTHER piece of rich meta (`cmd`, `cmd:args`, `agent:resume_flag`,
etc.) that only that function builds. Root-caused via the srv log: the
handler's own `tracing::info!(agent_id, "agent.open")` line — the very
first statement in `register_agent_open` — never appeared, even though
`~/.agentmux/logs/agentmuxsrv-*.log` clearly showed this exact block being
opened (agent name, ids, and downstream events like "reactive register
request" all present).

The actual cause: **the picker's "Continue" click never calls the
backend's `agent.open` RPC at all.** It goes through
`AgentPicker.tsx::handleReattach` → `agent-model.ts::launchAgentDefinition`,
which independently re-derives the *entire* launch — CLI args, env vars,
working directory, resume flags, output format — into its own `meta`
object and pushes it via `SetMetaCommand` + `ControllerResyncCommand`.
This is not a thin wrapper around the backend path; it is a second,
parallel implementation of the same job, apparently maintained
independently (their key sets have drifted to *almost* but not quite
identical field names). `register_agent_open`'s `agent.open` RPC exists
and works, but nothing in the primary UI flow calls it — it appears to
serve other callers only (e.g. MCP tool / programmatic opens).

**Collateral finding:** the existing, already-shipped `term:zoom`
persistence feature (`SPEC_AGENT_ZOOM_PERSISTENCE_2026_06_22.md`) has the
identical gap — it's seeded only in `register_agent_open`, so it was
likely never actually restoring zoom for picker-launched agents either.
Not confirmed with a live repro (out of scope to chase further here), but
the code shape is identical enough to flag.

**Fix:** duplicate the same seed logic into `launchAgentDefinition`
(`frontend/app/view/agent/agent-model.ts`), since that is the code path
that actually runs for the everyday flow:

- New `frontend/app/view/agent/agent-color.ts` — a TS counterpart of
  `agent_color.rs` (`pickAgentColor` FNV-1a hash over `TAB_COLORS`,
  `isValidAgentColor`, `dimAgentColor`). Deliberately NOT shared code with
  the Rust side — the two don't need to agree bit-for-bit, since whichever
  path assigns a color first persists it and every later reader (either
  path) just reads that stored value back.
- `launchAgentDefinition` already loads `contentMap` (all `AgentContent`
  rows for this agent) for config-file building — reads `ui:color`/`ui:zoom`
  from there directly, no new RPC round-trip needed for the read side. If no
  valid color exists, picks one and persists it via the existing
  `SetAgentContentCommand` RPC (best-effort — a failure doesn't block the
  launch). Adds `term:zoom` (mirroring `parse_seed_zoom`'s clamp/validate
  contract) and both frame color keys into the `meta` object already being
  built for `SetMetaCommand`.

This is a genuine duplicated-logic architecture smell worth a dedicated
follow-up (either the frontend should call the backend's `agent.open` RPC
instead of rebuilding launch meta itself, or the backend path should be
retired if it's truly unused by any real caller) — flagged here, not fixed
here; consolidating two independently-evolved ~150-line implementations is
a larger, riskier change than this feature justifies.

Verified live via CDP against a `task dev` build after the fix: opening a
never-before-opened agent through the actual picker UI produces a block
whose `.block-mask` inline style is `border-color: rgb(34, 197, 94)` —
exactly the agent's stored `ui:color` (`#22c55e`), confirmed by directly
reading the persisted block meta from the channel's SQLite store.

## 4. Out of scope (follow-ups, not this PR)

- Color picker in the create-agent pane UI (the 08-05 ask's UI half) — the
  shared `ColorSwatchPalette` component makes this cheap later.
- Showing the color in the agent picker / My Agents list (colored dot).
- An "agent color" editor anywhere in settings.

## 5. Test plan

- [x] Unit (Rust): migration assigns colors only to defs lacking one;
      id-hash pick is stable; invalid stored color is not seeded into
      block meta; a global-registry-only agent (no local row) still gets
      backfilled — the regression test for §3's `set_def_registry` fix.
- [x] Unit (TS): `pickAgentColor`/`isValidAgentColor`/`dimAgentColor` mirror
      the Rust suite (deterministic, palette membership, validation,
      dim-stays-valid).
- [x] Live, via CDP against a running `task dev` build:
      - Migration backfilled all 26 real agents on the test machine (read
        directly from `~/.agentmux/shared/agents/definitions/*.json`).
      - Opening a fresh agent through the **actual picker UI** (not a
        direct RPC call) renders the assigned color on the pane border —
        confirmed only after finding and fixing the §3.5 frontend-path gap.
      - Short-viewport / resize interactions unaffected (no relation to
        this feature's DOM surface).
- [ ] Not verified live: `term:zoom` restoration on the picker flow (the
      §3.5 collateral finding) — flagged, not chased down in this PR.

## 6. Files

| File | Change |
|---|---|
| `agentmux-srv/src/migrations/m0020_agent_color_backfill.rs` | New — backfill migration; attaches the global def registry itself (§3.2 note) |
| `agentmux-srv/src/migrations/mod.rs` | Register m0020 |
| `agentmux-srv/src/backend/agent_color.rs` (new) | Palette + id-hash pick + dim + validation, shared by migration/create/open |
| `agentmux-srv/src/backend/mod.rs` | Register `agent_color` module |
| `agentmux-srv/src/server/agent_handlers/core.rs` | Assign `ui:color` in `createagent` |
| `agentmux-srv/src/server/app_api/agent_open.rs` | Seed both frame color keys from `ui:color` (with assign-if-missing fallback) — the RPC path, not the one the picker uses (§3.5) |
| `frontend/app/view/agent/agent-color.ts` (new) | TS counterpart of `agent_color.rs` — §3.5 |
| `frontend/app/view/agent/agent-color.test.ts` (new) | Mirrors the Rust unit tests |
| `frontend/app/view/agent/agent-model.ts` | `launchAgentDefinition`: seed `term:zoom` + both frame color keys — the actual picker-flow fix, §3.5 |
| `frontend/app/block/blockframe.tsx` | Focused-state border-color memo: check `frame:activebordercolor` before `frame:hue` so the explicit "Pane Color" picker keeps working on agent panes (§3.4 note, reagent P1) |
