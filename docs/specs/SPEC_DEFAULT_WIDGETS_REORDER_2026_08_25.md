# SPEC: Default fresh-start widgets — Agent, Swarm, Armory, Sysinfo

**Date:** 2026-08-25
**Status:** proposed

---

## 0. Ask

> quick refinement: lets start the app (fresh widgets) with this order:
> Agent, Swarm, Armory, Sysinfo

Followed by: "update the docs too (anywhere there is talk of default
pinned widgets)".

---

## 1. Current behavior (audited against source, 2026-08-25)

Three genuinely separate mechanisms all currently encode the OLD 3-widget
default (`agent`, `sysinfo`, `swarm`) independently — none of them share
code with each other:

### 1.1 Window bootstrap / "Open another window" — backend Rust

`agentmux-srv/src/backend/wcore/mod.rs`:
- `seed_default_layout()` (144-166) creates 3 blocks (agent, sysinfo,
  swarm) then calls `write_default_three_pane_layout`. Used by first-launch
  bootstrap (`ensure_initial_data`).
- `default_three_pane_tree(agent_block_id, sysinfo_block_id,
  swarm_block_id)` (207-279) is the **pure geometry builder** — hardcoded
  to exactly 3 typed positional params, not a generic list. Current shape:
  ```text
    ┌────────────────┬──────────────┐
    │                │   sysinfo    │  size 2 of 10 ≈ 20%
    │     agent      ├──────────────┤
    │    (tall)      │              │
    │                │    swarm     │  size 8 of 10 ≈ 80%
    └────────────────┴──────────────┘
         50% width         50% width
  ```
- `agentmux-srv/src/server/service/window_create.rs:183-239` — the
  post-bootstrap "Open another window" path — has its OWN independent
  `for view in ["agent", "sysinfo", "swarm"]` loop, checks
  `seeded_ids.len() == 3`, then calls the same `default_three_pane_tree`
  positionally. Both paths call the same geometry builder (by design, so
  the tree shape itself can't drift) but each has its OWN copy of the
  3-item view list / block count.

### 1.2 New tab (existing window, "+" button) — frontend TS

`frontend/app/tab/tab-presets.ts:47-59` — `DEFAULT_TAB_PRESET`, a fully
declarative `PresetNode` tree (arbitrary N children, no hardcoded pane
count). Own doc comment: *"To change the defaults, edit **only** this
constant — the applier and every consumer of `createTab()` pick it up
automatically."* Currently: `agent` (left) | `sysinfo` / `swarm` (right,
stacked) — same 3 widgets, different mechanism, no code sharing with §1.1.

### 1.3 Widget bar "fresh-install defaults" — `widgets.json`

`agentmux-srv/src/config/widgets.json` — every widget entry carries
`display:pinned` (bool) and `display:order` (int). Consumed by
`frontend/app/window/action-widgets-config.ts`'s `getPinnedKeys()`
(92-116), whose own doc comment states: *"Not set → derive from
`display:pinned` in widget config... this is about **fresh-install
defaults**"* — i.e. the widget-bar's pinned icon row before a user (via
`settings["widget:pinned"]`) ever customizes it. This is a THIRD,
independent default, unrelated to §1.1/§1.2's *layout* defaults — it's
about which widgets are one click away in the picker, not what
auto-populates a fresh tab/window.

Current pinned set (`display:pinned: true`), in `display:order`:

| Order | Widget | Pinned |
|---|---|---|
| 1 | Agent | ✅ |
| 2 | Swarm | ✅ |
| 3 | Drone | ✅ |
| 4 | Warden | ✅ |
| 8 | Sysinfo | ❌ |
| 15 | Media | ✅ |
| 21 | Armory | ❌ |

Neither Sysinfo nor Armory is currently pinned; Drone, Warden, and Media
are pinned but not part of either layout default.

---

## 2. Design

**All three mechanisms updated to the same 4-widget set, same order:
Agent, Swarm, Armory, Sysinfo.** Not because they're the same mechanism —
they're not, and don't become one in this change — but because a user
reasonably expects "the default widgets" to mean the same thing whether
they're looking at a fresh window, a fresh tab, or the widget bar's pinned
row, and today those three surfaces already happen to agree by
coincidence (all three currently center on agent/sysinfo/swarm); keeping
that coincidental agreement intact after this change is a case of doing
the same job three times over consistently, not a design decision to
unify the mechanisms.

### 2.1 Layout geometry (§1.1, §1.2)

Extends the existing "Agent tall on the left, everything else stacked on
the right" shape from 2 stacked panes to 3, ordered top-to-bottom matching
the requested left-to-right/top-to-bottom reading order:

```text
  ┌────────────────┬──────────────┐
  │                │    swarm     │  size 4 of 10 ≈ 40%
  │     agent      ├──────────────┤
  │    (tall)      │   armory     │  size 4 of 10 ≈ 40%
  │                ├──────────────┤
  │                │   sysinfo    │  size 2 of 10 ≈ 20%
  └────────────────┴──────────────┘
       50% width         50% width
```

Sysinfo keeps its existing small (2/10) share — it's a compact status
readout, not a primary working pane, same reasoning the original 2-pane
split already established. The 8/10 previously all going to Swarm splits
evenly between Swarm and Armory (4/10 each) — both are primary, actively-
used panes; no stated reason to favor one over the other by size.

**Backend (§1.1):** `default_three_pane_tree` → renamed
`default_four_pane_tree`, gains an `armory_block_id: &str` fourth
parameter, tree gains a 4th child. `seed_default_layout` creates a 4th
block. `write_default_three_pane_layout` → renamed
`write_default_four_pane_layout`, same 4th param. `window_create.rs`'s
view array becomes `["agent", "swarm", "armory", "sysinfo"]`
(deliberately matching the target display order, not just "old order plus
armory appended" — makes the array self-documenting), `seeded_ids.len()`
check becomes `== 4`.

**Frontend (§1.2):** `DEFAULT_TAB_PRESET` — no structural rework needed,
the preset tree already supports arbitrary children; just add
`{ widget: "defwidget@armory" }` as a third child of the vertical split
and reorder to swarm/armory/sysinfo.

### 2.2 Pinned widget bar (§1.3)

`widgets.json`: Agent (order 1, pinned), Swarm (order 2, pinned), Armory
(order 3, pinned — was 21, unpinned), Sysinfo (order 4, pinned — was 8,
unpinned). Drone, Warden, Media move to `display:pinned: false` — the ask
named exactly 4 widgets as *the* default set, not "these 4 plus whatever
was already pinned"; keeping them pinned alongside the new 4 would leave
7 pinned widgets when the ask was for 4. Their own `display:order` values
are left as-is (5, 6, 16 respectively after renumbering makes room —
exact values only matter for their relative position in the unpinned
"More" list, not asserted by any test).

### 2.3 Widget defs already exist — no new registration needed

`armory` and `sysinfo` are both already fully defined in `widgets.json`
(`defwidget@armory` line 234, `defwidget@sysinfo` line 72) — this spec
only flips their `display:pinned`/`display:order` values and adds them to
the two layout-default mechanisms; no new widget/blockdef/icon work.

---

## 3. Out of scope

- No shared abstraction introduced between §1.1/§1.2/§1.3 — they stay
  three independently-encoded defaults, as they are today. Unifying them
  into one source of truth is a real, separate refactor opportunity, not
  requested here and riskier to land alongside a content change.
- No change to `display:order` semantics or the "More" bucket's own
  ordering logic — only which widgets are pinned and their relative
  order among themselves.
- Drone/Warden/Media's own widget definitions, icons, or availability are
  unchanged — only their pinned status.

---

## 4. Test plan

**Rust:**
- [ ] `seed_default_layout_creates_three_pane_layout` (renamed
      `..._four_pane_layout`) — asserts 4 blocks, views contains all of
      agent/swarm/armory/sysinfo, `leaforder.len() == 4`.
- [ ] `server/tests.rs`'s reducer-routed seed test — 4 seeded block ids,
      `default_four_pane_tree` called with 4 args, `leaforder.len() == 4`.
- [ ] `layout_seed_unknown_tab_errors` — updated call to
      `default_four_pane_tree("a","b","c","d")`.

**Frontend:**
- [ ] No dedicated test currently asserts `DEFAULT_TAB_PRESET`'s exact
      content (confirmed via grep) — none to update; the change is
      self-contained to the constant per its own doc comment.
- [ ] `action-widgets-config.test.ts` builds its own synthetic widget-map
      fixtures, not the real `widgets.json` — confirmed no test reads the
      real file's pinned set, so no update needed there either.

**Manual (`task dev`):**
- [ ] Fresh `~/.agentmux` (or a throwaway profile): confirm first launch
      shows Agent (left, tall) | Swarm / Armory / Sysinfo (right, stacked
      top-to-bottom).
- [ ] "Open another window": same layout.
- [ ] New tab ("+"): same layout.
- [ ] Widget bar: confirm Agent, Swarm, Armory, Sysinfo appear pinned in
      that order; Drone/Warden/Media have moved to "More".
