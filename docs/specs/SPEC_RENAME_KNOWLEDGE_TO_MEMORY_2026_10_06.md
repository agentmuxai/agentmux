# SPEC: Rename the Knowledge pane to Memory, and give "memory" one set of meanings

**Status:** active — §5 step 1 (the rename) is built in the PR that adds this spec; steps 2–4 follow. Decisions D1–D7 taken on the recommendations (operator, 2026-10-06).
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

**Agents take one bundle today.** The agent record holds a single bundle id, under its old name `memory_id` (`crates/srv/src/agents/types.rs`, `rpc_types/agent.rs`, `rpc_types/instance.rs`), and only Global Memory combines several bundles (`format_global_brain_block` joins them with `---`). Several bundles per agent is therefore a new capability, delivered as its own phase (§5 step 2):

- **Data:** the agent record gains an ordered `bundle_ids` list. `memory_id` stays readable for old records and clients, as the first entry, and a migration fills `bundle_ids` from it.
- **Launch:** combine bundles in the order picked. Instructions are joined as Global Memory joins its bundles. MCP servers and skills are the union, and when two bundles name the same server or skill, the first bundle wins and the launch notes the duplicate.
- **Everywhere one bundle is shown or passed today:** the Stash, the bundle summary, the launch modal, Drone nodes, ABF export of an agent, and the App API's agent fields. Each shows or takes the list.

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

It never adopted Knowledge and still says **Armory**: `FeaturesPage.tsx:60, 109, 182, 240, 365` and `Roadmap.tsx:21`. Update these to Connectors and Memory. `public/llms.txt:43` ("Armory Bundle Format") becomes Agent Bundle Format.

## 5. Delivery

1. **agentmux PR, the rename:** §3.1–3.5, §3.6's Identity removal and the field's new label, and §4.1, with tests. The existing tests cover the contract values; add migration tests for each old value in §3.2 (alias, section key, pinned key, hidden key, color key, command id, layout type).
2. **agentmux PR, several bundles per agent:** the rest of §3.6 (data, migration, launch combination, the list everywhere), with tests. The field becomes multi-select here.
3. **agentmux-docs PR:** §4.2 and §4.3, once (1) and (2) have merged, so the docs match the shipped UI.
4. **agentmux-landing PR:** §4.4.

## 6. Decisions

- **D1. Ids.** Rename the internal ids too, with the migrations in §3.2 (recommended); or rename labels and text only, keep `knowledge` internally, and need no migration at all. Keeping the old id means the code goes on calling the Memory pane "knowledge", which is the confusion this pass is meant to remove.
- **D2. The bundle-summary pane.** Remove it, so old blocks open the Memory pane (recommended); or keep it under a new id such as `bundle-summary`, labelled "Bundle".
- **D3. Docs URLs.** Swap the slugs as in §4.2 (recommended); or leave `/memory/` as Bundles and put the Memory pane at another slug. That would keep "memory" ambiguous on the site.
- **D4. Icon.** `brain` (recommended: the Personal section already uses it, and "book" reads as documentation); or keep `book`.
- **D5. No account.** Without the Identity field, use the provider's first account silently (recommended, it's today's default), or always start with no account and sign in at launch.
- **D6. Where else several bundles appear.** The launch modal and Drone nodes take the list too (recommended, one model everywhere), or only the new-agent form does.
- **D7. Duplicates across bundles.** First bundle wins, and the launch notes it (recommended); or refuse the combination when two bundles define the same MCP server or skill.
