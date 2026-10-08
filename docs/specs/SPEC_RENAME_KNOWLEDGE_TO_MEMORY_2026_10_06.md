# SPEC: Rename the Knowledge pane to Memory, and give "memory" one set of meanings

**Status:** active — §5 step 1 (the rename) shipped in PR #4425; step 2a (the backend for several bundles) in PR #4433; step 2b (the pickers) in PR #4434; steps 3 and 4 in agentmux-docs#157 and the landing site; the retired terms (§3.8) in agentmux-docs#158, the landing site and this repo; step 2c is cancelled: bundles no longer carry MCP servers (`SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md`). Decisions D1–D7 taken on the recommendations (operator, 2026-10-06), D8–D13 on 2026-10-07.
**Date:** 2026-10-06
**Author:** agent3 (Agent3@narko), at the operator's request
**Amends:** `SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md` (the pane's name; its §6 risk "'Knowledge' suggests retrieval (RAG) more than configuration … revisit only if users are confused" is that revisit)

## 1. The ask

The operator, 2026-10-06:

- Rename the **Knowledge** widget to **Memory**. "Memory" is what other agent platforms call this and is easier to understand; "Knowledge" sounds like a help-desk knowledge base.
- The agent Stash's **Memory** tab and this widget show the same memories, live in both, so they should share the name.
- Be thorough: code, comments and docs. Use the pass to make every doc consistent, because the names went through several rounds (Trust Center → Armory with a Memory tab → Knowledge → Memory) and are now jumbled, which confuses future agents.
- Fix the glossary in `agentmux-docs`.
- In the new-agent form, remove the **Identity** dropdown entirely, and turn the **Memory** dropdown into **Bundles**, a multi-select: an agent can take several bundles. A bundle is a collection of memory, skills and the rest (§3.6).

## 2. What "memory" means today

The word currently covers six different things, and the code, UI and docs use them inconsistently.

| Meaning | What it is | Where it lives | App API |
|---|---|---|---|
| **Global Memory** | Entries composed into every agent's startup instructions file (CLAUDE.md, GEMINI.md, …) at launch | Knowledge → Global (`GlobalBundleManager`, global bundle rows) | `GlobalMemoryList/Read/Write/Remove/History/Diff/Revert` |
| **Personal Memory** | One agent's own memory files, the provider's native memory folder (`<config>/projects/<name>/memory`, its `MEMORY.md` index), versioned by AgentMux | Knowledge → Personal (`NativeMemoryManager`) **and** the agent Stash's Memory tab. Same `agent:memory:*` data, live in both | `MemoryList/Read/Write/History/Diff/Revert` |
| **Bundle** | A package of instructions, MCP servers, memory and skills bound to an agent (formerly "Memory bundle", before that "Preset") | Knowledge → Bundles | — |
| The **`memory` view** | A read-only, per-agent bundle summary pane, label "Memory", title "Bundles" | `view/bundle/bundle.tsx` (`memoryPaneTab`) | — |
| **"Memory" form fields** | Pick a **bundle** for a new agent (new-agent form) or a Drone node | `AgentCreateFromTemplateModal.tsx:452`, `drone-view.tsx:917` | — |
| **Memory re-injection** | A hidden turn that puts Global and Personal Memory back into context after compaction | `memory-reinjection-controller.ts` | — |

Unrelated: process memory (RAM), e.g. the memory heartbeat. It stays as it is.

Comments also still describe Global and Personal as "the Armory 'Memory' tab" (`global-bundle-manager.tsx:4`, `global-bundle-model.ts:4`, `native-memory-manager.tsx`, `native-memory-history-model.ts:8`), from before Knowledge existed.

## 3. Design

### 3.1 The vocabulary after this change

| Term | Means | Notes |
|---|---|---|
| **Memory** (the pane) | The app-wide pane, formerly Knowledge, with four sections: **Global**, **Personal**, **Skills**, **Bundles** | Its Global section is Global Memory and its Personal section is Personal Memory |
| **Global Memory** | Unchanged | |
| **Personal Memory** | Unchanged. "Native memory" stays as the *code* name of its storage, never a UI name | The Stash's **Memory** tab is the per-agent view of it |
| **Bundle** | Unchanged: a collection of memory, skills, MCP servers and instructions, bound to an agent | The new-agent form's field is **Bundles** (§3.6); any single-bundle picker is labelled **Bundle** |
| **Memory re-injection** | Unchanged | |

The rule for any new text: **"Memory" alone means the pane.** Anything else is qualified as Global Memory, Personal Memory or "memory re-injection", or uses its own word (bundle, skill).

### 3.2 The pane is renamed everywhere, ids included

**Recommended (§6 D1):** rename the identifiers as well as the labels, so `knowledge` doesn't survive in code to mislead the next reader. Old values keep working through the existing migration mechanisms.

| Today | After | How old values keep working |
|---|---|---|
| view `knowledge` | view `memory` | Manifest `aliases: ["knowledge"]` (as `armory` has `["trust"]`). A stored `meta.view` resolves, it is not rewritten |
| meta `knowledge:section` | `memory:section` | `SectionPaneModel` reads the old key when the new one is missing, and writes only the new one |
| widget `defwidget@knowledge` | `defwidget@memory` | `getPinnedKeys` maps `knowledge` → `memory` at read time, as it maps `armory`; `isIndividuallyPinned` gets the same mapping |
| `pane:colors["knowledge"]` | `pane:colors["memory"]` | `widgetHueFor` falls back to the old key |
| command `app:knowledge` | `app:memory` | Old id kept as a `hidden: true` command, as `app:identity` is (macOS menu, user keybindings) |
| layout file `type: "knowledge"` | `type: "memory"` | Import accepts both; export writes `memory`. The armory migration (`armory_target`, `armoryMigrationPatch`) targets `memory` |
| `KNOWLEDGE_VIEW`, `openKnowledge`, `knowledgePaneTab`, `view/knowledge/` | `MEMORY_VIEW`, `openMemory`, `memoryPaneTab`, `view/memory/` | Code only |
| screenshot id `knowledge-bundles` | `memory-bundles` | Script only |
| icon `book` | `brain` (§6 D4) | |

### 3.3 The old `memory` view (bundle summary) goes away

The new pane needs the `memory` id. The bundle-summary pane holds it today, and nothing creates it any more: no widget, command, RPC or UI flow (`bundle-summary.tsx:37-40`). Only blocks in old workspaces and layout files with `type: "memory"` still reach it.

**Recommended (§6 D2):** remove `memoryPaneTab` and the `BundleView` it alone uses. An old block or layout entry with `view: "memory"` then opens the Memory pane, on its default section (Global). The summary's information is in its Bundles section, where the summary's own button already linked. `BundleSummaryPanel`, which the agent pane's Stash also uses, stays.

`widget:hidden@defwidget@<name>` keys need nothing: no code reads them any more.

### 3.4 Labels and text

- Pane label, tab pill, title (`Memory · Global`), aria label, widget bar, hamburger menu, command palette (keywords keep "knowledge" so the old name still finds it), macOS app menu (`Memory…`), Settings → Widget colors row, layout-import summary.
- Pointers: "Knowledge → Bundles", "Knowledge → Skills", "Knowledge → Personal", "Browse all in Knowledge →", "Manage/Edit in Knowledge" become Memory (`AgentNewBundleModal`, `AgentSkillsModal`, `AgentStartupModal`, `bundle-summary.tsx`, `bundle-manager.tsx` help text, `MemoryAdoptionApprovalWindow`).
- The new-agent form changes as in §3.6. The Drone node's "Memory" field becomes **Bundle**, or **Bundles** once agents take several (§6 D6).
- Agent-facing text: the `GlobalMemoryWrite` and `GlobalMemoryRemove` tool descriptions (`crates/mcp/src/tool_schemas.rs`), and the CLAUDE.md placeholder text that says to use "Knowledge -> Global" (`crates/srv/src/backend/providers.rs:888`). Files already written aren't rewritten; that's acceptable, the text stays understandable.
- Comments and test names that describe the pane (about 45 lines), including the stale "Armory 'Memory' tab" ones in §2.

### 3.6 The new-agent form

The form opened from a template card in the agent picker (`AgentCreateFromTemplateModal.tsx`):

- **Remove the Identity field.** Today it picks the provider account the agent signs in with, and preselects the provider's first account (`setAccountId(accounts()[0]?.id ?? "")`). Without the field, that first account is still used, silently; the account can be changed later in the agent's Stash (Accounts). With no account, the agent launches as today's "No auth" and signs in through the usual pre-launch sign-in panel.
- **Memory → Bundles, multi-select.** Pick any number of bundles, in order. None selected is today's "(vanilla CLI)".

**What "the agent's bundle" is today.** Not one thing. An inventory of the code found four per-agent bindings, plus the global ones (the same three-way split `ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md` §2.2 and `SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md` §1.3 call "unreconciled"):

| | Binding | Stored | At launch |
|---|---|---|---|
| A | The agent's **own** bundle ("&lt;Name&gt; — ABF"), created with the agent, never changed | `db_agents.default_memory_id`; `AgentDefinition.memory_id` on the wire | Its skills and MCP servers are added (`managed_union_bundle_refs`), and it gives the provider when the agent has none |
| B | The bundle **picked for a launch**: the new-agent form, the launch modal | `db_agents.memory_id`; `AgentInstance.memory_id` | **Nothing.** Only read back by `bundle.self.get` / `PresetGet` and the Continue and Recent lists |
| C | The **Startup** bundle, picked in the Stash's Startup tab | `db_agent_content`, content type `startup_bundle_id` | Its instructions are sent as the agent's first message (`sendStartupSequence.ts`) |
| D | A Drone node's bundle | the Drone graph's `agent_ref.memoryId` | Nothing: the runner ignores it |
| G | Global Memory (`is_global` bundles) | `db_bundles.is_global` | Composed into the startup file (`format_global_bundle_block`) |

Two bugs sit on the way: the UI launch path (`WriteAgentConfig`) never delivers a bundle's MCP servers, only the App API's `agent.open` does; and deleting a bundle leaves its id on agents.

**The design: one ordered Bundles list per agent.** What the user picks is what the agent gets.

- **The list is** the agent's own bundle (A) first, always, and not removable: it holds the skills and servers bound to just this agent. After it come the bundles the user picks, in order. A and the picks together replace B and C.
- **Stored** in a new per-channel table, `db_agent_bundles (agent_id, bundle_id, position)`, holding the picks only. The table is additive and created on every open, so the schema version isn't bumped and an older build still opens the database. A stays in `default_memory_id`, as today. `AgentDefinition` gets no new field (it is built in full in about a hundred places); the list has its own store calls and two RPCs, `getagentbundles` and `setagentbundles`.
- **Picked in** one control, `BundleListEditor`: the picks in order, each with move up, move down and remove, then an "Add a bundle…" select of the rest (§6 D13). It's used in the new-agent form (**Bundles**, sent with `agentdefcreatefromtemplate`'s `bundle_ids`), the launch modal (saved to the agent when Launch is clicked; a bundle is no longer required to launch), and the Stash, whose Startup tab becomes **Bundles** (saved on each change). The launch modal and the Stash show the agent's own bundle first, fixed.
- **At launch, in list order:**
  - **Instructions** go into the startup file, after Global Memory: one section per bundle with instructions, `# [Bundle] <name>` then the text, joined by the same `---` rule, reusing `global_bundle_sections`. They're no longer sent as a first message.
  - **Skills and MCP servers** are the union over the list, by id, as `managed_union_bundle_refs` does for A today. When two bundles name the same MCP server, the first wins, and the launch logs the duplicate.
  - **Provider:** unchanged: the agent's own, else its own bundle's (A). A picked bundle never changes the provider (§6 D11).
- **Deleting a bundle** removes it from this channel's lists. A launch skips a listed bundle that no longer exists (one deleted on another channel, or by an older build).
- **Compatibility.** `memory_id` (A) and B's `db_agents.memory_id` stay as they are; B goes on being written and read back (`bundle.self.get` / `PresetGet`, Continue, Recent) but still does nothing at launch. A `startup_bundle_id` (C) is folded into the list the first time the list is read, appended if it isn't there already, and the row removed. That one mechanism covers existing agents and a pick an older build writes later, so there is no separate data migration. The new-agent form's RPC (`agentdefcreatefromtemplate`) takes `bundle_ids`; `memory_id` is still accepted and means B, as today.
- **B is not migrated** (§6 D12).
- **Drone nodes** keep one inert `memoryId` and its **Bundle** label, because the runner doesn't use it (§6 D6).
- **Not in this change:** the cross-channel agent registry carries no bundle binding today (A has the same gap, issue #3148), so the list is per channel, like A. Memory re-injection after compaction re-delivers Global Memory only, not the picked bundles' sections; Claude Code re-reads the startup file itself, other providers lose them until the next launch.

### 3.8 Retired terms: "identity bundle" and "memory bundle"

Operator, 2026-10-07: the identity-bundle and memory-bundle concepts are over, so the terms go from user-facing text and code comments everywhere. They name nothing any more: an agent binds accounts directly, at most one per provider, and takes an ordered list of **bundles**.

| Term | Now |
|---|---|
| identity bundle, Identity bundle | the agent's **account** (or **accounts**); its **Identity** is that set |
| per-identity bundle (an auth dir under `<shared>/identities/<id>/`) | a **per-account auth dir** |
| memory bundle, Memory bundle | **bundle** |
| Global Memory bundle | a **Global Memory entry** |

**Kept, because they are literal names:**
- The retired tables `db_identity_bundles` and `db_memory_bundles`, in the migrations that still look for them. Comments that describe legacy rows refer to them by that table name.
- File names of dated specs that are cited, such as `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`.
- The retired migrations' own history comments (m0008, m0011–m0013), which record what they did to those tables.

**Not changed:** historical docs (dated specs, reports, archive, `VERSION_HISTORY.md`), as in §3.7.

**Also fixed in the sweep:**
- The README's alpha warning, which listed identity and memory bundles among the moving interfaces.
- The seeded "AgentMux Development" bundle's own text ("Select this Memory bundle"). It is seeded once, so only new installs get the new text.
- The comments about an `identitybundlebindings:changed` event that nothing publishes.
- The App API docs' identity-bundle RPC row, whose commands no longer exist.

### 3.7 What does not change

- The App API tool names (`MemoryWrite`, `GlobalMemoryWrite`, …), the `agent:memory:*` RPCs, and the Personal Memory storage.
- Historical docs: dated specs, retros, reports, analyses and `VERSION_HISTORY.md` stay as written (§4.1).
- Unrelated uses of the word "knowledge" (about 70 lines in code, more in docs): "acknowledge", "knowledge cutoff", "to my knowledge", third-party "knowledge graph".

## 4. Docs

### 4.1 agentmux

- `README.md`: line 60 still sends users to "the Armory tab"; line 175 describes "Accounts / Memories / MCP Servers / Skills / Startup" tabs. Update both to Connectors, Memory and the Stash.
- **Active specs that describe the current pane** get one dated amendment line each, not a rewrite: `SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md`, `SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md` (its color table row), `SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md`. Spec file names stay, because they are linked from `INDEX.md` and other specs.
- Everything else under `docs/` is historical, and left alone.

### 4.2 agentmux-docs (public docs site)

- **Slugs.** `/memory/` is today the **Bundles** page (`memory.md`, title "Bundles", 21 inbound links). Move it to `bundles.md` (`/bundles/`), then move `knowledge.md` to `memory.md` (`/memory/`). Redirect `/knowledge` → `/memory`. Old external `/memory/` links then land on the Memory pane page, whose Bundles section links to `/bundles/`. Keep the section anchors (`#global`, `#personal`, `#skills`, `#bundles`) so redirected deep links still match. Check whether the existing redirects (`/the-forge`, `/armory`) need new targets.
- **Pages** (about 65 lines): `knowledge.md`, `memory.md`, `pane-types.md`, `connectors.md`, `abf.md`, `first-agent.md`, `getting-started.md`, `installation.md`, `quickstart.md`, `main-menu.md`, `keybindings.md`, `warden.md`, `internals/agent-app-api.md`, plus the sidebar in `astro.config.mjs`.
- Also fix, in the same pass: `pane-types.md`'s "Memory (native/'Brain')" row and its "Hosts Identity and Memory subsections" line; the new-agent form's field described as **Memory** (`first-agent.md:59`, `getting-started.md:22`), which becomes **Bundle**; and `conversation-overhead.md:22`'s Global Memory row.

### 4.3 The glossary (`agentmux-docs/src/content/docs/glossary.md`)

Rewrite the memory-related and tab-related entries from §3.1, and fix what's wrong:

- **New entries:** Memory (pane), Global Memory, Personal Memory, Stash, Skills, **window tab**, **pane tab**, **document tab** (definitions from `SPEC_DOCUMENT_TABS_2026_10_02.md` §1: a window tab is a layout of panes; a pane tab is one block stacked in a pane; a document tab is one file inside one block, as in the Editor and Media panes; Browser has none).
- **Knowledge** becomes a redirect entry ("former name of Memory"). Its anchor `#knowledge` stays, so inbound links still resolve.
- **bundle:** keep "formerly Memory bundle, before that Preset", and say a bundle is picked in a **Bundle** field.
- **block** is wrong today. It's described as "an immutable persisted unit of pane state … terminal output, code block, diff, chat message — each is one or more blocks". In AgentMux a block is one view instance: one pane tab, with its own meta (`frontend/app/store/block-*`, `blockStack`). Correct it.
- Read every remaining entry against the code once, and fix any other drift found.

### 4.4 agentmux-landing

It never adopted Knowledge and still says **Armory** in its feature and roadmap copy. Update these to Connectors and Memory, and "Armory Bundle Format" to Agent Bundle Format.

## 5. Delivery

1. **agentmux PR, the rename:** §3.1–3.5, §3.6's Identity removal and the field's new label, and §4.1, with tests. The existing tests cover the contract values; add migration tests for each old value in §3.2 (alias, section key, pinned key, hidden key, color key, command id, layout type).
2. **agentmux PRs, several bundles per agent** (§3.6):
   - **2a, backend:** `db_agent_bundles`, the list RPCs (`getagentbundles`, `setagentbundles`), launch composition on both paths (`agent.open`, and `WriteAgentConfig`, which gains an optional agent id), skills and MCP over the list, and bundle and agent deletion. Inert until something writes a list, so it changes nothing on its own. With tests.
   - **2b, the switch:** the new-agent form's **Bundles** multi-select, the launch modal's list, the Stash's **Bundles** tab in place of Startup, the fold of `startup_bundle_id` into the list, and the first-message startup removed. These go together, so no build sends a bundle's instructions twice or loses a Startup pick. New pickers start empty. With tests.
   - **2c, a pre-existing bug:** the UI launch path delivers the bundles' MCP servers too. **Cancelled:** a bundle carries no MCP servers any more, so there is nothing to deliver (`SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md` §3.1).
3. **agentmux-docs PR:** §4.2 and §4.3, once (1) and (2) have merged, so the docs match the shipped UI.
4. **agentmux-landing PR:** §4.4.

## 6. Decisions

- **D1. Ids.** Rename the internal ids too, with the migrations in §3.2 (recommended); or rename labels and text only, keep `knowledge` internally, and need no migration at all. Keeping the old id means the code goes on calling the Memory pane "knowledge", which is the confusion this pass is meant to remove.
- **D2. The bundle-summary pane.** Remove it, so old blocks open the Memory pane (recommended); or keep it under a new id such as `bundle-summary`, labelled "Bundle".
- **D3. Docs URLs.** Swap the slugs as in §4.2 (recommended); or leave `/memory/` as Bundles and put the Memory pane at another slug. That would keep "memory" ambiguous on the site.
- **D4. Icon.** `brain` (recommended: the Personal section already uses it, and "book" reads as documentation); or keep `book`.
- **D5. No account.** Without the Identity field, use the provider's first account silently (recommended, it's today's default), or always start with no account and sign in at launch.
- **D6. Where else several bundles appear.** The launch modal and the Stash take the list too, so there's one model. Drone nodes keep their single, inert field: the runner ignores it, so a list there would promise something nothing does.
- **D7. Duplicates across bundles.** First bundle wins, and the launch notes it (recommended); or refuse the combination when two bundles define the same MCP server or skill.
- **D8. Bundle instructions** go into the startup file, after Global Memory, so they last through compaction like Global Memory does, rather than being sent as a first message.
- **D9. The Stash's Startup tab** becomes the agent's Bundles list, so a bundle is picked in one place.
- **D10. The agent's own bundle** stays, first and fixed, holding what's bound to just this agent.

- **D11. Provider.** Unchanged (the agent's, else A's). The provider is resolved on the shared agent registry, which can't see the per-channel list, and a picked bundle silently switching an agent's provider would surprise.
- **D12. Migrating B.** Not migrated. The form and launch modal preselected the first bundle on their own, and B never did anything at launch, so copying it would put an arbitrary bundle's instructions into every agent's startup file. Only C, which the user chose and which took effect, moves into the list.

- **D13. The multi-select is an ordered list**, not checkboxes, because order decides which bundle wins a clash. The app had no multi-select control; this one is built from the line-style `Select` and `IconButton`.

D8–D13 were taken on the recommendations (operator, 2026-10-07: "use best judgement").
