# Spec: drop the agent Stash, and give each of its tabs a home it already half has

**Status:** draft, awaiting the operator's approval. Decisions in §6 are marked for them, each with a recommendation.
**Date:** 2026-10-24
**Author:** AgentX@narko
**Task:** Spec #2850; the build is #2851 and starts after this is approved
**Requested by:** the operator, through AgentY: "i dont think we need the agent stash anymore, lets drop it"
**Verified against:** `agentmux` `main` @ `048917524`, by reading the code named below. Nothing here has been run.
**Supersedes:** [SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md](SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md) (the drawer as built, #3540)

## 1. Ask

Remove the Stash: the drawer that drops from the top of an agent pane, opened by the toolbox button (`title="Stash"`) at the right of the pane header. For each of its eight tabs, say where it goes or why it is dropped, and what replaces every way in.

## 2. What is there today

- **The drawer.** `AgentStashDrawer.tsx` wraps `AgentStashModal.tsx` (the name is historical: it no longer renders as a modal) in the shared `ResizableDetailsDrawer` (`anchor="top"`, height in block meta `agent:stashheight`). The tabs are a data list (`AgentStashModal.tsx:86-100`) rendered by `Tabs`; the drawer opens on Accounts, or on Personal Memory for a pane with no agent definition.
- **The one way in** is the header toggle, the only item of `endIconButtons()` (`agent-model.ts:183-228`), wired to the pane reducer's `StashExpand` / `StashCollapse` (`agent-view.tsx:193-208`). There is no command, shortcut, deep link, context menu item, widget SDK call, MuxAPI action or Operator Config text that opens it or one of its tabs.
- **Saved state.** None that matters. `stashOpen` lives in the pane reducer's memory and isn't in the snapshot (`useSnapshotPersistence.ts:132-134`); the selected tab is a local signal; `agent:stashheight` is block meta srv keeps as an unknown key; `localStorage["memoryEditor:split:stash"]` holds the Memory tab's editor split.
- **srv** has nothing named for the Stash. Some RPCs have the Stash as their only frontend caller today (§3); none is removed by this spec.

## 3. Each tab

| Tab | What it does | Where else it exists today | Goes to |
|---|---|---|---|
| **Accounts** | One agent's linked accounts (`agent-identity-links-panel.tsx`); Connect / Re-login for Claude only | **Connectors → Accounts**, per account: "Used by N agents", Bind to agent, Connect and Re-login for every provider. The agent pane's account notices already open it (`agent-view.tsx:1063`). No plain unbind. | **Connectors → Accounts.** Add **Unbind** to each agent in an account card's "Used by" list (`UnlinkAgentIdentityCommand`). Drop the per-agent table (D1). |
| **Seat** | The seat editor (`AgentSeatPanel.tsx`) | **My Agents → row menu → Seat…** opens the same panel (`MyAgentsList.tsx:600-611`) | **Dropped; already at parity.** Also added to the agent pane's right-click menu (§4). |
| **Personal Memory** | List, read, edit, history, diff, revert this agent's memory; **New file**; **Create MEMORY.md** | **Memory → Personal**, for every agent, with the same RPCs plus claims, adoption and launch cost. No create actions; `openMemory()` takes a section, not an agent. | **Memory → Personal.** Move **New file** and **Create MEMORY.md** there; `openMemory("personal", { agentId })` selects that agent; the agent pane's right-click menu gets **Personal memory** (§4). Panes with no definition (quick launch) must be listed there; the build checks and adds them if not (D3). |
| **MCP Servers** | This agent's bind state for each global server; Bind, **Unbind** | **Connectors → MCP servers**: create, edit, delete, Bind to agent, an in-use view from `list_for_agent`. No unbind (`mcp-model.ts:11-12` says unbinding lives in the Stash). | **Connectors → MCP servers.** Each server's detail lists its bound agents with **Unbind**. |
| **Skills** | The same, for skills; **Unbind** | **Memory → Skills**: CRUD, Bind to agent. No unbind, no per-agent list. | **Memory → Skills.** Each skill's detail lists its bound agents with **Unbind** (`skill.catalog.list_for_agent`, `skill.catalog.unbind`). |
| **Bundles** | Edit an existing agent's ordered bundle list (`AgentBundlesTab.tsx`, `BundleListEditor`) | **Nowhere else.** The new-agent form picks bundles at creation only; Memory → Bundles edits a bundle's contents. | **My Agents → row menu → Bundles…**, a modal with the same `BundleListEditor`, as Seat… is; also in the agent pane's right-click menu (D2). |
| **Registration** | Read-only: this agent's reactive registration, other instances claiming the same id, recent identity mismatches | **Warden → Host** lists local registrations for every agent, not the other-instance claims or mismatches. Warden → Audit hides register/unregister events, pointing at the Stash tab (`warden-audit-shared.ts:30-35`). | **Warden → Host.** An agent's row expands to its other-instance claims and recent mismatches (`reactive.registrations`); Warden → Audit stops hiding the register events (D4). |
| **Devices** | One checkbox, "Hide from paired devices" (`viewer.agent-hidden`) | **Nowhere else.** Settings › Paired devices lists and revokes devices and mentions hidden agents, with no toggle. | **Settings › Paired devices:** a "Hidden from devices" list with Add agent / Remove, over the same RPC (D5). |

So: Seat is the only tab with a full second home. The other seven move a missing piece (an unbind, a create action, one editor) into the surface that already owns that thing for every agent, which is where the operator looks for it anyway.

## 4. Ways in after the change

- **The header button goes.** `endIconButtons()` returns nothing for an agent pane; the header keeps its other controls.
- **The agent pane's right-click menu** (`getBodyContextMenuItems`, today Agent History and Quick Fork) gains **Seat…**, **Bundles…** and **Personal memory**, for the pane's own agent, so the three per-agent things a person reaches for from the conversation stay one click away (D6). Accounts already have their way in: the pane's account notices open Connectors.
- **My Agents' row menu** gains **Bundles…** beside Seat….
- Nothing else opened the Stash, so nothing else changes.

## 5. What the build removes, keeps and updates

**Removed:**
- `AgentStashDrawer.tsx`, `AgentStashModal.tsx`, `AgentStashModal.scss`, `styles/_stash-drawer.scss` (and its `@use` in `agent-view.scss`), the leftover `AgentStashModal.scss` import in `modal-dispatch.tsx`.
- The reducer's `stashOpen` and `StashToggle` / `StashExpand` / `StashCollapse` (`agent-pane-state/types.ts`, `reducer.ts`), the model's three callback slots (`agent-model.ts:92-94`), the wiring and drawer in `agent-view.tsx`.
- The tab components whose content moves: `AgentMcpModal.tsx` + `agent-mcp-model.ts`, `AgentSkillsModal.tsx` + `agent-skill-model.ts`, `AgentDevicesPanel.tsx`, `AgentRegistrationPanel.tsx` (+ `.scss`), and `AgentNativeMemoryModal.tsx` with its two models once Memory → Personal has the create actions. Before deleting each `.scss`, every class it defines is grepped across the frontend and scripts, and rules a surviving file uses move to that file's own stylesheet.
- Dead code the Stash had already replaced: the `agent-identity` and `agent-memory` modal kinds (`modal-layer.ts:61-89`, `modal-dispatch.tsx:68-90`), `AgentIdentityPanel.tsx` (reachable only through them) and its `@container agent-stash` rule, and the empty `BACKDROP_DISMISSIBLE_KINDS`.
- `components/AgentStashDrawer.tsx` from `agent-view-sources.ts`.

- The **Environment** tab (`AgentEnvPanel` mounted as a Stash tab; added by #5479 after this spec was written) leaves with the drawer. The panel itself and **My Agents → row menu → Environment…** stay: that is its home.

**Kept:**
- `ResizableDetailsDrawer` (the Shell drawer uses it), `AgentSeatPanel`, `BundleListEditor`, `agent-identity-links-panel.tsx` (the `identity` pane tab still mounts it), `AgentPrimitiveModal.scss` and `AgentNativeMemoryModal.scss` if a surviving component still uses them.
- Every RPC: each Stash-only one gets its new caller (`skill.catalog.list_for_agent` / `unbind` and `mcp.catalog.unbind` in the detail views, `getagentbundles` / `setagentbundles` in Bundles…, `viewer.agent-hidden` in Settings, `reactive.registrations` in Warden → Host). No binding changes, so no mobile or MuxAPI caller can break.

**Saved state:** nothing to migrate. `agent:stashheight` stays an ignored meta key (srv keeps unknown keys; layout files never write it). `memoryEditor:split:stash` is left behind in local storage: a few bytes, removed by nothing.

**Updated:**
- Comments that name the Stash in srv (`app_api/mcp.rs`, `app_api/skill.rs`, `native_memory_handlers.rs`, `bundle/*`, `storage/agent_bundles.rs`, `storage/skills.rs`, `rpc_types/commands.rs`) and the frontend (`mcp-model.ts:11-12`, `skill-model.ts:11-12`, `AgentCreateForm.tsx:249`, `theme.scss`, `ui.scss`, `Tabs.tsx`, `mixins.scss`, `floating-ontop.tsx`, `agent-media.tsx`, `types.ts`).
- `README.md` (the Stash paragraphs, lines ~61, 215, 241).
- Screenshot and recording scripts: the `agent-stash` shot (`scripts/ui-screenshots/agent-shots.mjs`) and the `stash-tour` clip with `STASH_TABS` (`recordings.mjs`), with their tests (`recorder.test.mjs`, `publish.test.mjs`).
- Specs that treat the Stash as current UI get a one-line pointer to this spec: the seats spec (§3 names the Stash's Seat tab as the editor), the Codex sign-in spec (plans Stash Connect for Codex: that goes to Connectors), and `docs/specs/INDEX.md`.
- The docs site (`agentmux-docs`): grep for "Stash" and move each mention to its new home, in the same change set as the build.

## 5b. The six unused starter skills (folded in, from the token report; task #2969)

A fresh install seeds six global skills (`config/starter-skills.json` through `m0015_seed_starter_skills`): systematic-debugging, tdd, code-review, commit-hygiene, verification-before-completion and security-basics. They are bound to no agent, and the Skills tab and Memory → Skills count them as "0 skills" for an agent. They are still written into agents' `.claude/commands` or `.claude/skills` and listed to the model each session, so every session pays for them and none uses them.

- **Recommendation (D7): remove them.** Stop seeding (retire `m0015`, `skill_seed.rs`'s seeding and `config/starter-skills.json`; keep `starter_skill_id` only if `mcp_seed.rs` still needs it, which the build checks), and add a one-time cleanup migration that deletes a starter skill only when it is bound to no agent and its content still equals the shipped text (matched by its deterministic id and a content hash). A skill the operator edited, bound or renamed stays. Writing skills out to an agent stays what it is for bound skills only.
- **Honest count.** Before the build changes the count, the build finds why an unbound skill is listed to agents at all (the writer in `agent_config.rs` iterates the skills it is given; the build traces which list it is given and whether global skills are included by default) and fixes that at its source, so "0 skills" and what an agent is shown agree, for starter skills and for any other unbound global skill.
- **Not chosen:** keeping them as opt-in. Nothing has used them since they were seeded, and the operator can add any skill from Memory → Skills.
- Tests: a fresh install seeds no skills; the cleanup removes an untouched starter skill and keeps an edited, a bound and a renamed one; running it twice changes nothing; an agent's written files and the count agree for a bound and an unbound skill.
- Ships in phase 2 with the removal (it needs the Skills surface settled), as its own commit.

## 6. Decisions for the operator

| # | Decision | Recommendation |
|---|---|---|
| **D1** | Accounts | **Connectors → Accounts only**, with Unbind added to each card's "Used by" list. No per-agent account table anywhere. |
| **D2** | Where an existing agent's bundle list is edited | **My Agents → Bundles…** and the agent pane's right-click menu, a modal like Seat…. (Alternative: a "Used by" list with Add/Remove in Memory → Bundles, which loses the order the editor keeps.) |
| **D3** | Personal memory | **Memory → Personal**, which gains New file and Create MEMORY.md and can open on one agent; the agent pane's right-click menu opens it there. |
| **D4** | Registration diagnostics | **Warden → Host**, an agent's row expands to its claims elsewhere and recent mismatches. (Alternative: drop them: they are diagnostics for a rare problem.) |
| **D5** | "Hide from paired devices" | **Settings › Paired devices**, a list of hidden agents. |
| **D6** | What replaces the header button | **Nothing in the header.** Seat…, Bundles… and Personal memory join the agent pane's right-click menu; accounts are reached through the pane's own notices and Connectors. |
| **D7** | The six starter skills | **Remove them**: stop seeding, and a one-time cleanup that deletes only untouched, unbound ones (§5b). |

## 7. Phases

Nothing is ever missing: the new homes land before the drawer goes.

1. **New homes** (one PR, or two if it is large): Unbind in Connectors (accounts, MCP) and Memory → Skills; New file and Create MEMORY.md in Memory → Personal, with `openMemory` taking an agent; Bundles… in My Agents; the hidden-agents list in Settings; claims and mismatches in Warden → Host, and Audit no longer hiding register events; the right-click menu items. Each with its tests.
2. **Remove the Stash** (one PR): everything in §5's removed list, the comment, README and script updates, and the spec pointers. The docs site change in the same change set.

Each PR has a changeset (phase 1 `minor`, phase 2 `minor`: a visible control goes away).

## 8. Tests

- **Phase 1:** each new control writes through its RPC and updates its view (an Unbind in Connectors MCP, Memory → Skills and Connectors Accounts; New file and Create MEMORY.md in Memory → Personal; Bundles… saves an ordered list; the hidden-agents list adds and removes; Warden → Host shows a claim and a mismatch from a stubbed `reactive.registrations`); `openMemory("personal", { agentId })` selects that agent; the right-click menu shows the three items for an agent pane and none on a history tab. Tests move with the code they cover: `AgentBundlesTab.test.tsx`, `AgentDevicesPanel.test.tsx`, `AgentNativeMemoryModal.test.tsx` become tests of the new homes.
- **Phase 2:** the header has no Stash button (an `endIconButtons` test); the reducer has no `stashOpen` (`reducer.panels.test.ts`'s Stash block goes); `AgentDrawers.test.tsx` keeps only the Shell drawer; `agent-view-sources.test.ts` passes without the entry; a block saved with `agent:stashheight` opens normally; knip finds no orphaned file; `ClaudeLoginPanel.test.tsx`'s stand-in modal stops naming the Stash.
- **Live** (my own `task dev`, with `AGENTMUX_HOME_OVERRIDE`): every new home on screen, and an agent pane with no toolbox button. No Kanban actions from the dev build until the work-store isolation fix lands.

## 9. Not in this spec

- The `identity` pane tab (`identity-pane.tsx`), which still mounts the accounts table but that no flow creates: a separate cleanup if wanted.
- Per-agent private MCP servers (`mcp-model.ts` mentions them; nothing builds them).
- The `memoryEditor:split:stash` local storage key.

Agent: AgentX@narko
