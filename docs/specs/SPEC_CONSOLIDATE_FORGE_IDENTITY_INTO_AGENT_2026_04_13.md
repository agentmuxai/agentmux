# SPEC — Consolidate Forge + Identity into the Agent Pane

**Date:** 2026-04-13
**Status:** Draft (revised)
**Owner:** AgentA
**Related:**
- `SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md` (per-agent identity plumbing)
- `SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md` (completed — agent-view.tsx is now 297 lines of composition)

---

## 1. The observation

We have three widgets that all revolve around the same mental model — "what agent are you running":

| Widget | Purpose |
|---|---|
| `defwidget@agent` | Pick an agent to launch, run it, display activity |
| `defwidget@forge` | CRUD for forge agent definitions (name, icon, prompt, tools, skills) |
| `defwidget@identity` | Manage account bindings (GitHub, AWS, git author, SSH) |

Both Forge and Identity are **per-agent concerns** — they exist to answer "how is *this specific agent* configured." The natural place to edit them is next to the agent itself.

The user's instruction: each agent card in the picker gets two extra buttons — **Forge** (configs other than identity) and **Identity** (accounts / git author / SSH). Clicking either opens that agent's settings inline. The top-level `forge` and `identity` widgets go away.

## 2. Goals

1. **Per-agent, button-driven.** Every entry in the agent picker gets two extra buttons: **Forge** and **Identity**. Clicking one opens a panel scoped to *that* agent — not to a global edit screen.
2. **Remove `defwidget@forge` and `defwidget@identity`** from the widget bar. Forge/Identity are no longer top-level surfaces.
3. **Zero feature loss.** Everything the old Forge / Identity widgets could do, you can now do from the per-agent button. Creating a new agent (the only non-per-agent Forge action) gets a "+ New agent" tile in the picker.
4. **Zero backend change.** All RPCs stay as-is (`ListForgeAgents`, `SetForgeAgent`, identity account commands). This is a pure frontend reorganization.

## 3. Non-goals

- **Not merging the domain models.** ForgeViewModel and IdentityViewModel stay as distinct TS classes — they already work and their RPC plumbing is fine.
- **Not introducing per-agent identity binding.** That's `SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md`. This spec creates the *place* where that binding UI will live; the binding itself is a later PR.
- **Not redesigning the Forge form.** `ForgeForm` / `ForgeDetail` / `ForgeSkillForm` keep their current layouts. We just render them inside a per-agent slide-out instead of a top-level widget.
- **Not touching anything after launch.** The buttons only exist on the picker screen. Once an agent is running, `AgentPresentationView` is unchanged.

## 4. Target UX

### 4.1 The agent card grows two buttons

Current `AgentCard` (from `AgentPicker.tsx`):

```
┌────────────────────────────────────────┐
│ 🤖  claude-sonnet                      │
│     general-purpose coding agent       │
└────────────────────────────────────────┘
```

New layout — buttons inline on hover, compact on the right:

```
┌────────────────────────────────────────┐
│ 🤖  claude-sonnet             [⚙][👤] │
│     general-purpose coding agent       │
└────────────────────────────────────────┘
                                  │  └── Identity button — opens IdentityPanel for this agent
                                  └───── Forge button — opens ForgePanel for this agent
```

- Clicking the card body (anywhere except the buttons) still launches the agent — unchanged.
- Clicking **⚙ (Forge)** opens an inline slide-down/slide-over panel with `ForgeDetail` / `ForgeForm` preloaded for that specific agent.
- Clicking **👤 (Identity)** opens an inline slide-over with `IdentityPanel` scoped to that agent's account bindings.
- Pressing Esc or clicking the card header again collapses the panel.

Event propagation: button clicks call `e.stopPropagation()` so they don't accidentally launch the agent.

### 4.2 The slide-over panel (single instance)

At most one settings panel is open at a time. Structure:

```
┌──── AgentPicker ─────────────────────────────┐
│  [🤖 claude-sonnet   [⚙][👤]]                │  ← the card is now "expanded"
│  ┌─────────────────────────────────────────┐ │
│  │ Forge: claude-sonnet            [Close] │ │  ← slide-over header
│  │ ─────────────────────────────────────── │ │
│  │  (ForgeForm for this agent)             │ │
│  │                                         │ │
│  │  name: claude-sonnet                    │ │
│  │  icon: 🤖                                │ │
│  │  prompt: ...                            │ │
│  │  tools: [...]                           │ │
│  │  skills: [...]                          │ │
│  │                                         │ │
│  │           [Cancel] [Save]               │ │
│  └─────────────────────────────────────────┘ │
│                                              │
│  [🤖 reviewer-bot  [⚙][👤]]                  │  ← other cards still listed below
│  [🤖 auditor       [⚙][👤]]                  │
└──────────────────────────────────────────────┘
```

Visual treatment: the expanded panel lives **immediately below the card it belongs to**, pushing the cards below it down. Other cards stay interactive (user can click another card's ⚙ to swap the panel's target). Clicking outside any card or pressing Esc collapses it.

Rationale for inline (vs overlay modal): the agent picker is already a list of cards; inline expansion keeps the user's mental map. No context loss.

### 4.3 "+ New agent" tile

The only action the old Forge widget had that isn't per-agent is **creating a brand-new agent**. Add a `+ New agent` tile at the bottom of the picker list (or top, TBD). Clicking it opens the same slide-over panel but in "create mode" — `ForgeForm` with empty fields. On save, the new agent joins the list.

This tile replaces the "Open Forge" call-to-action in the current empty-state UI.

### 4.4 Running agents

No change. Once `block.meta.agentId` is set, `AgentPresentationView` takes over and the buttons + slide-over UI are never rendered. The buttons belong to the picker state only.

### 4.5 State persistence

The expanded panel's state (which agent is expanded, which tab — Forge vs Identity) is **session-local**, not persisted to block meta. Rationale: these are ephemeral editing sessions, not a view the user wants to return to on next launch. Close pane → reopen → you're back on the plain picker.

## 5. Component structure

```
frontend/app/view/agent/components/
├── AgentPicker.tsx                     REFACTOR — owns the expanded-panel
│                                       state (`expandedAgentId`, `expandedTab`)
│                                       and renders <AgentCard> per agent plus
│                                       <NewAgentCard> at the end.
├── AgentCard.tsx                       NEW — extracted from the inline card
│                                       JSX currently inside AgentPicker's
│                                       <For>. Owns click-to-launch + the two
│                                       settings buttons. Takes
│                                       { agent, expanded, expandedTab,
│                                         onLaunch, onToggleForge,
│                                         onToggleIdentity, onCollapse }.
│                                       Renders the expanded slide-over as a
│                                       direct child when `expanded` is true.
├── AgentCardSettingsPanel.tsx          NEW — the slide-over content. Switches
│                                       on `tab` prop: renders
│                                       <ForgePanel agent={agent} /> or
│                                       <IdentityPanel agent={agent} />.
│                                       Owns the [Close] button and Esc key
│                                       handling.
├── NewAgentCard.tsx                    NEW — "+ New agent" tile. Clicking
│                                       opens AgentCardSettingsPanel in
│                                       create mode (no agent passed).
│
frontend/app/view/forge/components/
├── ForgePanel.tsx                      NEW — the Forge form, takes
│                                       { agent?: ForgeAgent, onSave,
│                                         onCancel }. agent=undefined means
│                                       create mode.
└── (existing ForgeForm / ForgeDetail / ForgeSkillForm / ForgeContentSection
   unchanged — already accept a model/agent)
│
frontend/app/view/identity/components/
└── IdentityPanel.tsx                   NEW — the Identity UI, takes
                                        { agentId: string } and shows the
                                        account bindings scoped to that
                                        agent. For now it can render the
                                        current global IdentityView body
                                        unchanged; per-agent scoping lands
                                        in SPEC_FORGE_AGENT_IDENTITY.
```

## 6. Forge / Identity as sub-panels (not widgets)

The key move: `ForgePanel` and `IdentityPanel` are plain subtree components, not widget views. They don't need a `ViewModel` registered with `BlockRegistry`. They're simpler than the current `ForgeView` / `IdentityView` wrappers because they don't have to own a block — the enclosing agent pane already owns one.

**Model access inside the panels:**
- `ForgePanel` talks directly to `RpcApi.ListForgeAgentsCommand` / `RpcApi.SetForgeAgentCommand`. No ForgeViewModel needed. The panel receives a single agent (or undefined for create mode) and emits an `onSave` / `onCancel` callback. The parent AgentPicker re-fetches its list on save via the existing `forgeagents:changed` event subscription.
- `IdentityPanel` does the same with the identity RPCs. Same pattern: props in, callbacks out, no embedded VM.

This is a simplification over the previous draft, which proposed instantiating `ForgeViewModel` / `IdentityViewModel` inside the agent pane. Those wrapper classes exist today because the old widgets needed a `ViewModel` interface to register with `BlockRegistry` — now that we're not a top-level widget, we don't need that indirection. The panels become plain SolidJS components with RPC calls in `onMount`.

If some existing logic lives inside `ForgeViewModel` (e.g. form validation, dirty-tracking), that logic moves into `ForgePanel` itself or into a small `useForgeAgentForm` hook next to it. Same for `IdentityViewModel`.

## 7. Extraction sequence

Four PRs, each shippable on its own.

### PR 1 — Extract `AgentCard` + `NewAgentCard` from `AgentPicker`

**Scope:** Pure refactor inside `frontend/app/view/agent/components/`. The inline `<button class="agent-card">` currently inside the `<For>` moves into `AgentCard.tsx`. Add a placeholder `NewAgentCard.tsx` at the end of the list (no action yet — just the tile). No behavior change, no new features.

**Risk:** Very low.

**Verification:** Open agent pane → picker still looks the same, still launches agents, now shows an extra "+ New agent" tile that does nothing when clicked.

### PR 2 — Add the per-card Forge / Identity buttons + `ForgePanel` component

**Scope:**
- Add ⚙ and 👤 buttons to `AgentCard.tsx` with `e.stopPropagation()` so they don't trigger launch.
- Create `forge/components/ForgePanel.tsx` — the form UI, props-driven, no viewmodel. Extract the body of the current `ForgeForm` or `ForgeDetail` as the starting point.
- Create `AgentCardSettingsPanel.tsx` that conditionally renders `<ForgePanel>` or a stub for Identity.
- Clicking ⚙ on a card expands the panel inline below it with the Forge tab active.
- `NewAgentCard` now opens the settings panel in create mode.
- At most one card expanded at a time. Clicking ⚙ on another card swaps the target. Esc / Close button / clicking outside collapses.

**Risk:** Medium. This is the biggest PR. It introduces the slide-over UX and the ForgePanel component together. We can split it further if review gets large (2A = buttons + panel skeleton; 2B = ForgePanel wire-up).

**Verification:**
- Click ⚙ on an agent → panel expands with that agent's current data → edit → save → panel collapses → card shows the new data.
- Click + New agent → panel opens empty → fill in → save → new agent appears in the list.
- Click ⚙ on agent A, then ⚙ on agent B → panel swaps target without a flash.
- Esc collapses.
- Clicking the card body (not the button) still launches — the existing launch path is untouched.

### PR 3 — Wire the Identity tab + `IdentityPanel`

**Scope:**
- Create `identity/components/IdentityPanel.tsx`. Initially it just renders the existing `IdentityView` body unchanged (still showing global account bindings) — per-agent scoping is out of scope for this PR; it's the follow-up in `SPEC_FORGE_AGENT_IDENTITY`.
- Wire the 👤 button in `AgentCard` so it opens the panel with the Identity tab.
- `AgentCardSettingsPanel` switches on tab (Forge / Identity) via a tab switcher inside the panel header, so a user who opened the Forge tab can flip to Identity without collapsing first.

**Risk:** Low. The Identity UI is simpler than Forge (fewer fields) and we're not yet doing per-agent scoping.

**Verification:** Click 👤 on an agent → panel opens with Identity tab → see the current Identity view rendered inside → flip tab to Forge → see the Forge form for the same agent → flip back → state preserved during the session.

### PR 4 — Remove the old widgets + migrate existing panes

**Scope:**
- Remove `defwidget@forge` and `defwidget@identity` from `agentmux-srv/src/config/widgets.json`.
- Unregister `ForgeView` and `IdentityView` in `frontend/app/block/block.tsx`.
- Delete `forge-view.tsx` and `identity-view.tsx` (the top-level View wrappers). Their internals — `ForgeForm`, `IdentityView` body, etc. — stay where they are and are now consumed by `ForgePanel` / `IdentityPanel` instead.
- **Migration:** on block load, if a saved block has `view: "forge"` or `view: "identity"`, rewrite it to `view: "agent"` (landing on the picker). Simpler than the tab-based migration in the earlier draft — there's no tab state to preserve since the settings panels are session-local.
- Audit every string reference to `"forge"` / `"identity"` as a view name:
  - `pkg/wconfig/defaultconfig/widgets.json`
  - `frontend/app/block/block.tsx`
  - Any "open X" helpers in `frontend/app/store/global.ts`
  - Keyboard shortcut / menu hooks

**Risk:** Medium — the migration is the only spot this can regress. Dry-run against a test layout with both an open Forge pane and an open Identity pane before merging.

**Verification:**
- Fresh install: widget bar has `agent`, no `forge`, no `identity`.
- Existing install with a Forge or Identity pane open: on next startup, those panes become plain agent picker panes. The user's data (forge agents, identity accounts) is untouched because it lives in backend state, not pane meta.
- No dead code references to `forge` or `identity` as view names.

## 8. Risks and open questions

### 8.1 ForgePanel without a ForgeViewModel

We're replacing a ViewModel with prop-driven components. Before PR 2, audit `ForgeViewModel` to see what state it actually holds:
- Is it just a thin wrapper around atoms + RPC calls? → easy migration, move logic into the panel.
- Does it own form state (dirty tracking, validation, undo)? → that logic moves into a small `useForgeAgentForm` hook colocated with `ForgePanel`.
- Does it subscribe to wave events? → subscription moves into the panel's `onMount`.

If the ViewModel turns out to carry more than expected, PR 2 grows and we split it into 2A/2B.

### 8.2 Layout: panel-pushing-cards

When an expanded panel pushes the cards below it down, the user's scroll position changes. We should:
- Either scroll the expanded card to the top of the viewport on expand, so it's visually anchored.
- Or animate the expansion so the motion is clearly "this panel grew," not "the list jumped."

Go with scroll-into-view on expand — standard pattern, no surprise.

### 8.3 "At most one expanded" state

`expandedAgentId: string | null` lives in `AgentPicker` as a local signal. Switching targets is a single `setExpandedAgentId(otherId)` call. No reducer needed.

### 8.4 Keyboard: Esc handling

A global `keydown` listener on the agent pane would conflict with the existing `useAgentKeyboard` (Ctrl+B/Ctrl+F). Cleanest: add Esc handling *inside* `AgentCardSettingsPanel` via its own onMount, scoped to focus — only fires if the pane has focus and the panel is open. Remove the listener when the panel closes.

### 8.5 Delete / rename

The old Forge UI had explicit "delete" and "rename" affordances on its list. Those need a home in the new UX:
- **Delete:** a red "Delete agent" button at the bottom of the expanded ForgePanel. Confirmation dialog required.
- **Rename:** already covered — editing `name` in the Forge form is the rename action.

Agents currently running (block meta has their `agentId`) shouldn't be deletable while running; disable the delete button and show a tooltip.

## 9. Estimated cost

| PR | Est. active time | Review rounds |
|---|---|---|
| 1. Extract AgentCard + NewAgentCard | 20 min | 0–1 |
| 2. Buttons + ForgePanel + slide-over | 90 min | 1–2 |
| 3. IdentityPanel + tab switcher | 45 min | 0–1 |
| 4. Remove widgets + migration | 60 min | 1 |
| **Total** | **~3.5 hours** | **2–5** |

Realistic wall-clock: 1–2 days serialized through reagent review. Each PR shippable in isolation.

## 10. Success criteria

After PR 4:

- Widget bar has one agent-related entry.
- Every agent card in the picker has two buttons (⚙ Forge, 👤 Identity) next to the launch area.
- Clicking either button opens an inline panel scoped to that specific agent.
- The panel has a tab switcher so users can flip between Forge and Identity without collapsing.
- Creating a new agent is done via a "+ New agent" tile at the end of the list.
- Deleting an agent is done from the expanded Forge panel.
- All Forge CRUD operations work identically to the old widget.
- All Identity operations work identically to the old widget.
- Existing layouts with Forge / Identity panes migrate cleanly on next startup.
- `agent-view.tsx` stays at 297 lines. All new code lives in `components/` files.

---

## 11. Deltas from the earlier draft

The previous version of this spec proposed a **top-level tab bar** on the picker (Agents / Forge / Identity). That was rejected by the user: per-agent buttons are simpler, and Forge/Identity are inherently per-agent concerns, not top-level modes.

Key changes:
- **No tab bar on the picker.** The agent list stays as-is; settings are per-card.
- **No ForgeViewModel / IdentityViewModel wrappers inside the agent pane.** Plain props-driven components.
- **"+ New agent" tile** handles the one non-per-agent Forge action (create).
- **Session-local expansion state** — no block-meta persistence.
- **Migration simpler** — no tab-state preservation, just "was a Forge/Identity pane → becomes an Agent pane."

## 12. What this is NOT doing

- **Not deleting Forge or Identity functionality.** Every capability is preserved.
- **Not changing any RPC or backend command.** Pure frontend reorganization.
- **Not touching `agent-view.tsx` beyond composition.** The running-agent surface is unchanged.
- **Not implementing per-agent identity binding.** That's the follow-up spec. This one creates the place where that binding will live.
