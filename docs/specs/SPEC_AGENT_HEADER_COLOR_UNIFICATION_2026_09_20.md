# SPEC: decommission the env-var pane color system; header inherits the agent's identity color

**Date:** 2026-09-20
**Status:** implemented in #3452 — see §4 for exactly what shipped
**Author:** Camper
**Related:** `docs/specs/SPEC_AGENT_COLOR_2026_08_08.md` (the persisted per-agent
`ui:color`/`frame:activebordercolor` system this now also feeds the header),
`docs/specs/SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23.md` (Draft, 2026-06-23
— documents `AGENTMUX_AGENT_COLOR` as part of its original design; that
assumption is now stale, see §5), `docs/specs/SPEC_PANE_COLOR_SYSTEM_CONSOLIDATION_2026_09_20.md`
(the larger, not-yet-implemented follow-up this spec's own findings fed into)

---

## 1. What existed (three color systems, one of them entirely undocumented)

Investigating a user report ("agent panes get a unique border color, but
their header remains black") surfaced three independent color mechanisms,
not two:

1. **Per-agent identity color** — `frontend/app/view/agent/agent-color.ts`
   / `agentmux-srv/src/backend/agent_color.rs`. FNV-1a hash of the agent id
   onto a 14-color palette, persisted as `ui:color` (agent_content) and
   seeded into `frame:activebordercolor`/`frame:bordercolor` block meta at
   launch (`SPEC_AGENT_COLOR_2026_08_08.md`). **Border only** — never fed
   the header.
2. **The "Pane Color" right-click picker** — `frontend/app/block/pane-color-menu.ts`.
   12 hues, writes `frame:hue` block meta, drives both header bg
   (`hueToHeaderBg`) and border (`hueToActiveBorder`/`hueToBorder`).
   Explicit, user-chosen, already correctly touched both surfaces.
3. **An undocumented, older env-var-driven system** —
   `frontend/app/block/autotitle.ts`'s `detectAgentColor`/`detectAgentTextColor`.
   Read `AGENTMUX_AGENT_COLOR`/`AGENTMUX_AGENT_TEXT_COLOR` from the block's
   `cmd:env` (populated by the OSC-16162 shell-integration prompt hook —
   `agentmux-srv/src/backend/shellintegration/{bash,zsh,fish,pwsh}.{sh,fish,ps1}`
   — see `frontend/app/view/term/termosc.ts`'s `E` command), falling back to
   a hardcoded table for the legacy fixed agent names (`Agent1`–`Agent5`,
   `AgentA`, `AgentX`, `AgentY`, `AgentG`). **Header only.** Predates system
   #1 and was never reconciled with it — `SPEC_AGENT_COLOR_2026_08_08.md`
   never mentions it, and its own design doc
   (`SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23.md`, June 2026) documents
   it as the header's *base* layer, underneath the hue picker.

This is why the header and border visibly disagreed: the border read #1,
the header read #3, and nothing connected them.

## 2. Decision: decommission #3, unify header onto #1

Per direct user request. Rather than reconciling three systems, remove the
oldest/narrowest one and have the header read the same value the border
already does.

**Why decommission rather than keep as a fallback:** #3's env-var plumbing
(shell integration → OSC-16162 → `cmd:env` → `detectAgentColor`) only ever
existed to solve "give the header a color," which #1 already solves more
completely (persisted, backfilled for every existing agent, not dependent
on a shell session actually reporting env back). Keeping both meant two
sources of truth for the same concept with no defined precedence — worse
than picking one.

## 3. What was actually removed (full scope, not just the frontend read side)

Traced every consumer before removing anything — this reached further than
the two frontend functions:

| Layer | What | File |
|---|---|---|
| Frontend detection | `detectAgentColor`, `detectAgentTextColor`, `AGENT_COLOR_ENV_VAR`, `AGENT_TEXT_COLOR_ENV_VAR`, `DEFAULT_AGENT_COLORS`, `DEFAULT_AGENT_TEXT_COLORS` | `frontend/app/block/autotitle.ts` |
| Frontend consumption | `agentColor`/`agentTextColor` memos (header); `blockAgentColor`'s `detectAgentColor` call (border) | `frontend/app/block/blockframe.tsx` |
| Backend env passthrough | `AGENTMUX_AGENT_COLOR`/`AGENTMUX_AGENT_TEXT_COLOR` removed from the pane env allowlist | `agentmux-srv/src/backend/pane_env.rs` |
| Backend cleanup | `env_remove` calls for the same two keys (now removing an unset var — dead) | `agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs` |
| Shell integration | `COLOR:` field dropped from the change-detection string; the conditional `AGENTMUX_AGENT_COLOR` JSON payload field removed | `bash.sh`, `zsh.sh`, `fish.fish`, `pwsh.ps1` under `agentmux-srv/src/backend/shellintegration/` |

**Deliberately NOT removed:** `AGENTMUX_AGENT_ID` and `detectAgentFromEnv` —
a separate, still-used feature (pane title detection from env, including the
"Terminal" pseudo-agent guard). `AGENTMUX_AGENT_TEXT_COLOR` was never
actually emitted by the shell scripts (only `AGENTMUX_AGENT_COLOR` was) —
it only ever reached the frontend via a user manually exporting it, so its
removal is allowlist/detection-side only, nothing to touch shell-side.

## 4. What shipped alongside the decommission

Three additional changes, all part of the same PR:

1. **Header now reads `frame:activebordercolor` when no explicit hue is set**
   (`blockframe.tsx`'s `headerStyle` memo) — the direct fix for "border is
   colored, header is black." Text color computed generically via a new
   `pickReadableTextColor` (WCAG relative luminance, `#000`/`#fff`) instead
   of #3's hardcoded per-agent text-color table.
2. **`blockAgentColor` (the border memo for a plain terminal with agent env,
   not a real `view: "agent"` pane) now reads the same persisted
   `frame:activebordercolor`** instead of the removed `detectAgentColor` —
   consequence: a plain terminal pane that only has `AGENTMUX_AGENT_ID`
   manually exported (never actually launched as a real agent, so never
   seeded with `frame:activebordercolor`) no longer picks up a color from
   that alone. Only real agents get a color now.
3. **Every non-agent pane (`view !== "agent"`) with no other color source
   gets one fixed header color** (`NON_AGENT_DEFAULT_HEADER_BG`,
   `hsl(220, 12%, 16%)`) instead of the default near-black
   (`--block-bg-solid-color: rgb(0,0,0)`). A single constant, not
   per-pane-type or randomized — user request, same session.
4. **The Haiku-derived per-turn activity summary
   (`term:ambient_summary`, `useAgentActivitySummary.ts`) no longer renders
   in the agent pane header** (`agent-model.ts`'s `viewText` now always
   returns `[]`; the now-unused activity-flash signal and its `createEffect`
   were removed too). The *generation* itself is untouched — `swarm-model.ts`
   still reads the same meta key for the Swarm view, so the ambient-summary
   pipeline stays live for that consumer.

## 5. Stale doc flagged, not fixed here

`SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23.md` (Status: Draft) documents
`AGENTMUX_AGENT_COLOR` as the header's base layer in its §7 interaction
table — now inaccurate. Left as Draft rather than retroactively edited;
`SPEC_PANE_COLOR_SYSTEM_CONSOLIDATION_2026_09_20.md` is the actual current
design going forward and supersedes it in practice, but a formal
Status-line update to that June doc is a small follow-up, not bundled here.

## 6. Verification

- `cargo build -p agentmux-srv` clean; targeted tests
  (`pane_env`, `agent_id_for_jekt_tests`) pass, including the updated
  `other_global_cmd_env_keys_remain_injectable` (swapped its example var off
  the now-meaningless `AGENTMUX_AGENT_COLOR`).
- `npx vitest run frontend/app/block/autotitle.test.ts` — 38/38 pass, no
  changes needed (none of the removed functions had direct test coverage).
- `npx tsc --noEmit` — zero errors in any touched file (one pre-existing,
  unrelated error elsewhere in the tree, confirmed present before this PR).
- Full-repo grep for `AGENTMUX_AGENT_COLOR`/`AGENTMUX_AGENT_TEXT_COLOR`
  across `.rs`/`.ts`/`.tsx`/`.sh`/`.fish`/`.ps1`: zero live references
  remain, only explanatory comments in the three files this spec touched.
- **Not verified live** (no `task dev` build run this session, by explicit
  user choice) — the actual rendered header colors were reasoned from code,
  not screenshotted. Flagged, not glossed over.
