# SPEC: consolidate pane/tab color systems — persist explicit agent-pane picks, unify the tab-select indicator

**Date:** 2026-09-20
**Status:** proposed — nothing in this document has shipped
**Author:** Camper
**Trigger:** direct user request, same session as
`SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md` (that spec's decommission
work surfaced most of the inventory this one builds on)
**Related:** `docs/specs/SPEC_AGENT_COLOR_2026_08_08.md`,
`docs/specs/SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md`,
`docs/specs/SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23.md` (Draft, stale
per the header-unification spec §5), `docs/specs/SPEC_COLOR_PALETTE_EXPANSION_REUSE_2026_06_30.md`
(the tab-color swatch picker's own origin, referenced but not re-read in
full here)

**Note on this document's own status:** written and kept local per updated
workflow guidance (2026-09-20, same session) — specs are for local use and
land bundled with their implementing PR, not pushed standalone. This one has
no implementation yet, so it has not been committed.

---

## 1. Full inventory (verified against code, not assumed)

Four color-storage locations exist today, at two different scopes (block vs.
tab), with inconsistent wiring between them:

| # | Key | Scope | Set by | Read by | Status |
|---|---|---|---|---|---|
| 1 | `frame:hue` | block meta | `pane-color-menu.ts`'s `setHue` (right-click "Pane Color" picker, any pane type) | `blockframe.tsx`: `headerStyle` (bg), `computeFocusRingBorderColor` (border, both focus states) | Live, explicit, ephemeral (block-scoped — lost if the pane closes and the agent reopens in a new block) |
| 2 | `frame:activebordercolor` / `frame:bordercolor` | block meta | Seeded at agent launch from persisted `ui:color` (agent_content) — `SPEC_AGENT_COLOR_2026_08_08.md` | `computeFocusRingBorderColor` (border); `headerStyle` (bg, as of today's header-unification work) | Live, per-agent-persistent, now drives both surfaces |
| 3 | `tab:color` | **tab** meta | `tab.tsx`'s own color-swatch picker (`TAB_COLORS`, a **different** palette from either of the above) | `tab.tsx`/`tab.scss` only — sets `--tab-color`, styles the tab strip's own background/active-bar | Live, but **completely disconnected** from #1/#2 — a colored agent pane's own tab does not reflect the pane's color unless separately hand-picked |
| 4 | `bg:activebordercolor` / `bg:bordercolor` | tab meta | **Nothing.** Declared in `frontend/types/srv-types.d.ts`; read in `computeFocusRingBorderColor` (highest-priority tier, above both #1 and #2); grepped the entire repo (frontend + `agentmux-srv`) for a writer — none exists | `computeFocusRingBorderColor` | **Dead.** Either a removed feature whose reader was never cleaned up, or a planned-but-never-wired one. Always `undefined` in practice today. |

A fifth, env-var-driven system existed at the block/header level until
today's companion spec decommissioned it — not re-described here.

## 2. The three things asked for

### 2.1 Header inherits the agent's color (delivered already)

The header-unification spec's shipped work already covers this: when no
explicit `frame:hue` is set, the header now reads the same
`frame:activebordercolor` the border does. An agent with no explicit pick
already shows a colored header today, post that PR. Restated here only so
this document's scope is clear: this spec is about the *remaining* two
asks, not re-litigating the one already closed.

### 2.2 An explicit pick on an agent pane should persist to the agent, not just the pane

**Current behavior:** `setHue(blockId, hue)` writes `frame:hue` on the
**block**. If an agent's pane is closed and the agent is later reopened
(a new block, per the container-restart-control-gap report's finding that
this is common even for host-type agents via any close+reopen cycle), the
launch-time seed from `ui:color` re-applies and the previous explicit hue
pick is gone. Contrast: `frame:activebordercolor` (system #2) is
re-seeded from persisted `ui:color` on every open specifically *because*
it's meant to follow the agent — `frame:hue` has no equivalent persistence
path.

**Ask:** picking a hue via the header context menu on an **agent** pane
(`view === "agent"`) should persist to that agent's own `ui:color`
(the same `agent_content` row `SPEC_AGENT_COLOR_2026_08_08.md` already
established as the source of truth for "this agent's color"), not just
`frame:hue` on the current block.

**Design options:**

1. **Write `ui:color` in addition to `frame:hue`.** `setHue` gains an
   agent-aware branch: when the target block is `view === "agent"`, also
   issue the existing `SetAgentContentCommand` RPC (`agent-model.ts` already
   uses this — see `SPEC_AGENT_COLOR_2026_08_08.md` §3.5) to persist the
   picked hex, converting the hue to a concrete `#rrggbb` via
   `hueToActiveBorder(hue)` (the same conversion the border already uses)
   before writing. `frame:hue` stays as the *immediate* block-level
   override (still wins over the freshly-written `ui:color` on THIS block,
   per the existing priority chain) — the persistence is for the *next*
   open, not a retroactive rewrite of `frame:activebordercolor` on other
   already-open blocks for the same agent.
2. **Write `frame:activebordercolor`/`frame:bordercolor` directly instead
   of `ui:color`.** Cheaper (one block-meta write, no RPC round-trip to
   agent_content), but only fixes THIS block — doesn't survive the agent
   being reopened in a new block, which is the actual complaint ("changes
   ... should persist"). Rejected on that basis unless "persist" is
   confirmed to mean "for this pane's lifetime" rather than "for this
   agent, always" — needs explicit confirmation, not assumed (§4).
3. **Retire `frame:hue` for agent panes entirely, route all agent-pane
   picks through `ui:color`.** Simplest end state (one persistence path
   for agent panes) but a bigger behavior change — an agent pane's color
   would no longer have a "temporary, this-pane-only" override mode at
   all. `frame:hue` would remain for non-agent panes (nothing else to
   persist to).

**Recommendation: option 1.** Smallest change, keeps the existing
block-level override semantics intact for the current session, adds
persistence without removing a capability. Non-agent panes are
unaffected — `frame:hue` stays purely block-scoped for them, since there's
no agent identity to persist to.

### 2.3 The selected-tab indicator should match the pane's resolved color

**Current behavior:** a tab's own "selected" bottom-accent color comes
purely from `tab:color` (system #3), set only via the tab strip's own
separate swatch picker. It has no relationship to what color the pane(s)
inside that tab are actually showing (`frame:hue`/`frame:activebordercolor`).
A tab holding a vividly-colored agent pane shows a plain/default tab
indicator unless the user *also* separately picks a tab color.

**Ask:** the tab-select indicator should reflect "the combined system" —
i.e., derive from the same resolved pane color, not require a second,
independent pick.

**Open design question, not resolved here:** a tab can hold more than one
block (a stacked pane / tab group) — whose color wins if the tab contains
multiple differently-colored panes? Candidates:
- The **focused/active** block within that tab (changes as focus moves
  between stacked panes — dynamic, possibly distracting).
- The **first/primary** block (stable, but "primary" isn't a concept that
  exists today — would need defining).
- Only apply this when the tab holds exactly one block; fall back to
  `tab:color`/default for stacked tabs.

**Recommendation:** start with the last option (single-block tabs only) —
covers the common case (one agent per tab) without needing to invent a
new "which block owns the tab's color" rule for the stacked case, which
can be a defined follow-up once the simple case ships and the actual
stacked-tab color question gets its own explicit decision.

**Mechanically:** `tab.tsx` would need to read the tab's single block's
resolved color (`frame:activebordercolor`/`frame:hue`-derived hex) the same
way `computeFocusRingBorderColor` does, as a fallback below explicit
`tab:color` (explicit user pick on the tab itself should still win — same
"explicit beats derived" precedent every other tier in this system already
follows).

## 3. Dead code found, worth removing regardless of the above

`bg:activebordercolor`/`bg:bordercolor` (inventory #4): declared, read,
never written, anywhere. Two options: remove the read path (and the type
declarations) as unused, or — if there's a plan for this that predates
current staff's knowledge — leave a comment and confirm before deleting.
Given zero writers exist across the entire repo, "unused" is the
better-supported read; recommend removing in whatever PR implements §2,
since touching `computeFocusRingBorderColor` for §2.3 is already in scope
there.

## 4. Open questions requiring explicit confirmation before implementation

1. **§2.2 option 1 vs. the alternative reading of "persist."** Confirm
   "persist to memory" means "follow this agent across future opens"
   (→ `ui:color`/agent_content) and not merely "survive this pane's own
   reload" (→ block meta already does this passively, nothing to build).
2. **§2.3's stacked-tab color source**, if/when that case is addressed
   beyond the single-block-tab recommendation.
3. **Whether to remove `bg:activebordercolor`/`bg:bordercolor`** (§3) in
   the same PR, or file it as a separate, even-smaller cleanup.

## 5. Explicitly out of scope

- Re-deciding anything already shipped in
  `SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md` (the env-var
  decommission, the non-agent fixed header color, the haiku-summary
  removal).
- Adding a `ui:color` picker anywhere outside the existing right-click
  header menu (e.g. the agent creation flow) — `SPEC_AGENT_COLOR_2026_08_08.md`
  §4 already flagged that as a separate follow-up, still true here.
- Live UI/CDP verification of any of this — nothing in this document has
  been implemented, so there is nothing to verify yet.
