# SPEC: Retire the Armory — two panes, Connectors and Knowledge

**Status:** active — decided (§9). Shipped: Phase 1 (the two panes, entry points, migration, default widget bar), #4345; Phase 2 (server and agent-facing text), #4347. Remaining: Phase 3 (removing the Armory shim and `app:identity`, a release later), §5
**Date:** 2026-10-05
**Author:** Camper (agent), at operator request
**Related:** `ARCHITECTURE_ARMORY_2026_07_20.md` (what the Armory holds and why),
`SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md` (the decided Bundle/Memory vocabulary and
its pending Phase 4 UI fold, which this would replace), `SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30.md`
(the current Memory section), `SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md`.

---

## 0. Summary

**The ask (operator, 2026-10-05):** move Memory out of the Armory, and retire the Armory in
favour of two tabs. In the terms of §0.1, those are two **pane tab** types: two panes.

**Decided (operator, 2026-10-05):**

| Pane | What it's for | Sections |
|---|---|---|
| **Connectors** | what agents connect to, outside AgentMux | **Accounts** (sign-ins: Claude, Codex, GitHub, Google, Slack, AWS, AgentMux Cloud, …) · **MCP servers** |
| **Knowledge** | what agents know and carry | **Global** · **Personal** · **Skills** · **Bundles** |

"Connectors" is the word Claude and ChatGPT use for connections to outside services, and they
aren't all MCP: a GitHub or Google connector is often a plain OAuth integration. The Armory's
Accounts section already holds exactly those (§3), so it becomes a section of Connectors, next
to MCP servers.

**Recommendation: do it.** The Armory is a thin shell (about 170 lines) around six
self-contained managers. Splitting it is mostly recomposition, entry points and migrating
saved state, not a rewrite. Two plainly named panes say what's inside, where "Armory" has to
be learned. It also fixes a real gap: today nothing that opens the Armory can pick a section,
so every "Armory → Accounts" or "Armory → Bundles" link lands on whatever section was used
last (§1.2).

The same day the operator also set the direction for bundles: a bundle packages instructions,
MCP servers, memory and skills, isn't tied to a harness, and can be bound to any agent, and ABF
becomes the **Agent Bundle Format**, cut as v0.3 (§4.6, §4.7). The public docs need a matching
pass (§5.1).

## 0.1 Which "tab"

AgentMux has three kinds of tab (`SPEC_DOCUMENT_TABS_2026_10_02.md` §1), and this spec uses
the words strictly:

| Kind | What it is | This spec |
|---|---|---|
| **Window tab** | a whole layout of panes, at the top of the window (Tab 1, Tab 2) | unchanged |
| **Pane tab** | one pane (one block, one view) stacked in a pane slot, in the pane header | the Armory is one pane tab type today; **Connectors and Knowledge become two pane tab types**. "Pane" below means one of these |
| **Document tab** | one document inside a pane (Editor files, Media files) | not used |

The parts of each pane (Accounts and MCP servers; Global, Personal, Skills and Bundles) are
**sections**: a navigation inside one pane, like the Armory's rail today. They aren't document
tabs (none of them is a document) and they aren't pane tabs (they share one block).

## 1. The Armory today

One pane tab (`view: "armory"`, alias `"trust"`, `frontend/app/view/armory/`), with a left
rail of five sections. Each section is an existing manager component, always mounted and
shown or hidden by a CSS class (`armory-view.tsx`):

| Section | Component | What it holds |
|---|---|---|
| Accounts | `AccountsManager` | sign-ins to outside services (`accounts-catalog.ts`): AgentMux Cloud, GitHub, Google, AWS, OpenAI, Anthropic, Slack, a custom bearer token, and the model providers' CLI logins |
| Memory → Global | `GlobalBundleManager` | workspace-wide instructions composed into every agent's startup file (global bundles) |
| Memory → Personal | `NativeMemoryManager` | what each agent writes about itself (`MemoryWrite`), with version history |
| Skills | `SkillManager` | the skills catalog |
| MCP Servers | `McpManager` | MCP server configs |
| Bundles | `BundleManager` | reusable agent definitions (instructions, skills, MCP refs), and `.abf` import |

The selected section is block meta `armory:section`; the Memory section's scope (Global or
Personal) is `armory:memory:subsection` (`armory-model.ts`).

### 1.1 How it's opened

| Entry point | Where |
|---|---|
| Widget bar button "Armory" (pinned, vault icon) | `crates/srv/src/config/widgets.json` (`defwidget@armory`); a click always creates a new block |
| Hamburger menu → Armory | `frontend/app/window/hamburger-menu.tsx` |
| Command palette "Identity & Memory" (`app:identity`) | `frontend/app/store/command-registry.ts` |
| macOS app menu "Identity & Memory…" | `crates/cef/src/macos_menu.rs` |
| Agent pane failure row: auth with nothing to bind, usage limit ("Armory (switch / upgrade)"), spawn failure | `frontend/app/view/agent/failure/failure-accessory.ts`, via `useAgentFailure.ts` |
| Stash dialogs: "Browse the Armory catalog →", "edit it there" | `AgentMcpModal.tsx`, `AgentSkillsModal.tsx` |
| Startup dialog "Armory → Bundles" | `AgentStartupModal.tsx` |
| Bundle summary pane "Edit/Manage in Identity & Memory" | `frontend/app/view/bundle-summary.tsx` |
| Layout files (type `"armory"`, `config.section`) | `crates/srv/src/backend/layout_file.rs` |

No MCP or App API tool opens the Armory.

### 1.2 Three problems with it as it is

1. **The name has to be learned.** "Armory" says nothing about accounts or memory, and two
   older surfaces still call it "Identity & Memory" (the command palette and the macOS menu),
   so the same place has two names.
2. **Links can't land on a section.** Every entry point opens the Armory without choosing a
   section (`openOrFocusPaneByView("armory")` with no block definition, or a block with only
   `view: "armory"`). It shows the last-used section, defaulting to Accounts. So "Armory →
   Accounts" in an auth error, "Browse the Armory catalog" in the Skills dialog and "Armory →
   Bundles" in the Startup dialog only work if the user happened to leave the Armory there.
3. **The widget button doesn't focus an existing Armory;** each click opens another one.

## 2. Why two panes

- **They're the two jobs.** People go to the Armory either to connect something (sign in to a
  provider or service, bind an account to an agent, add an MCP server) or to look at or change
  what agents are told, remember and can do. Two panes make each one click away and nameable in
  an error message ("open Connectors").
- **Outside versus inside.** Connectors holds the links to things outside AgentMux; Knowledge
  holds what lives inside it and goes into an agent. Every section falls clearly on one side.
- **It matches the other panes the widget bar opens,** which are named for what they show:
  Agent, Swarm, Sysinfo, Files.
- **It fixes §1.2's link problem by construction:** each pane takes a section in its block
  definition, so every link can say where it goes.

## 3. Names

**Connectors.** In Claude and ChatGPT, a connector is any connection to an outside service:
some are MCP servers, many are plain OAuth integrations (GitHub, Google Drive, Slack). The
Armory's Accounts section is already that list: AgentMux Cloud, GitHub (OAuth or token), Google
Workspace, AWS, OpenAI, Anthropic, Slack and a custom bearer token, alongside the model
providers' logins. MCP servers are connections to outside services too. So both go in one pane
under the industry's word, as two sections:

- **Accounts:** sign-ins. The word stays as the section name because that's what each entry is
  (an account you sign in to and bind to agents), and it keeps "Bind an account" phrasing
  true.
- **MCP servers:** the MCP server configs, as today.

**Knowledge.** What an agent knows and carries: instructions for every agent (Global), what each
agent writes itself (Personal), Skills, and Bundles that package them (§4.6). It also respects
the 2026-09-09 vocabulary, where **Memory** means only native memory: Personal is that memory,
`MemoryWrite` and the other memory tools stay native-memory-only, and the pane itself isn't
called "Memory". A bundle's MCP servers are references to connectors, so a bundle sits in
Knowledge and points across to Connectors.

Names considered, for the record:

| For | Name | Why not |
|---|---|---|
| the first pane | Accounts | narrower than what it holds once MCP servers join; kept as a section |
| the second pane | Memory | stretches the 09-09 word over Skills and Bundles |
| the second pane | Loadout | needs a moment the first time |
| the second pane | Kit, Bundles, Context | says little; Global and Personal aren't bundles; "context window" |
| the second pane | Brain, Stash | retired (09-09); taken (the per-agent Stash dialog) |

## 4. Proposed design

### 4.1 Two pane tab types

| Pane | View id | Label | Icon | Sections (block meta) | Default |
|---|---|---|---|---|---|
| Connectors | `connectors` | Connectors | `plug` | `connectors:section`: `accounts` · `mcp` | `accounts` |
| Knowledge | `knowledge` | Knowledge | `book` (or `brain`) | `knowledge:section`: `global` · `personal` · `skills` · `bundles` | `global` |

Both view ids are unused today. (The id `memory` would have been taken anyway: it's the
per-agent bundle summary pane, `frontend/app/view/bundle/bundle.tsx`, a persisted value the
09-09 spec froze.) Section labels: Accounts, MCP servers; Global, Personal, Skills, Bundles.

Each pane's section nav is the Armory's existing nav pattern (the Memory pane's Global/Personal
sub-nav), and the managers mount and hide exactly as today: `AccountsManager` and `McpManager`
in Connectors; `GlobalBundleManager`, `NativeMemoryManager`, `SkillManager` and `BundleManager`
in Knowledge.

**Bundles in Knowledge** (decided). A bundle packages an agent's instructions, MCP servers,
memory and skills (§4.6); three of those live in Knowledge, and its MCP servers are references
to Connectors. It's also where `.abf` files are imported. Choosing a bundle for an agent stays
where it is today (the launch dialog, the agent picker, Stash → Startup). The 09-09 Phase 4
idea of folding Global into Bundles as a scope filter still applies, now within one pane, and
can follow later.

### 4.2 Entry points

Every entry point opens the right pane and section:

| Today | After |
|---|---|
| Widget button "Armory" | two widgets, **Connectors** and **Knowledge**, each focusing an existing pane of that type before creating one; both pinned by default (§4.8) |
| Hamburger → Armory | two items: Connectors, Knowledge |
| Palette / macOS menu "Identity & Memory" (`app:identity`) | `app:connectors` "Connectors" and `app:knowledge` "Knowledge"; palette search also finds them by "Accounts", "MCP", "Memory", "Skills", "Bundles"; keep `app:identity` as a hidden alias that opens Connectors, for one release |
| Failure row "Armory → Accounts", usage limit, spawn failure | "Open Connectors → Accounts" |
| Stash: Skills "Browse the catalog", "edit it there" | Knowledge → Skills |
| Stash: MCP "Browse the catalog", "edit it there" | Connectors → MCP servers |
| Startup dialog "Armory → Bundles"; bundle summary pane | Knowledge → Bundles |

`openOrFocusPaneByView` needs a way to pass the section when it focuses an existing pane (set
`connectors:section` or `knowledge:section` on the focused block), which is the piece that
fixes §1.2's link problem.

### 4.3 Migrating saved state

Users have Armory blocks in their window tabs (in open pane slots, as pane tabs), in saved tab
presets and in layout files. None of them should break or fall back to a blank pane:

- **Blocks with `view: "armory"` or `"trust"`.** On load, rewrite each one by its saved
  section:
  - `accounts` (or unset) → `view: "connectors"`, `connectors:section = accounts`;
  - `mcp` → `view: "connectors"`, `connectors:section = mcp`;
  - `memory` / `native_memory` / `skills` / `bundles` → `view: "knowledge"` with
    `knowledge:section` set (`memory` plus `armory:memory:subsection` → `global` or
    `personal`; `native_memory` → `personal`).

  Keep `term:zoom`. This follows the legacy-value pattern already in `armory-model.ts`
  (`native_memory`).
- **Layout files.** `layout_file.rs` reads type `"armory"` with `config.section`, and maps it
  the same way. It writes the new types (`"connectors"`, `"knowledge"`, each with
  `config.section`). Update `schema/agentmux-layout.v1.schema.json` to accept both.
- **Widget pins.** A `widget:pinned` setting that names `armory` becomes `connectors` +
  `knowledge`. `defwidget@armory` stays readable for one release.
- **Unchanged:** `term:zoom`, the per-manager localStorage keys
  (`memoryEditor:split:armory-*`), and the `written_by = "armory-ui"` value on memory version
  rows. They're internal, and renaming them buys nothing.

### 4.4 Text

| Where | Change |
|---|---|
| Frontend strings: the Armory label, failure rows, Stash and Startup dialogs, `AgentNewBundleModal`, `bundle-manager`, `BundleImportSelectModal`, `MemoryAdoptionApprovalWindow`, `useAgentControllerStatus` ("Sign in… from Armory → Accounts") | name the pane and section: "Connectors → Accounts", "Connectors → MCP servers", "Knowledge → Skills" |
| Spawn error text, `crates/srv/src/identity/resolver/errors.rs` ("Bind an account for this provider in the Armory.") | "…in Connectors → Accounts". **Must change in the same commit as the matcher in `crates/srv/src/agents/failure.rs`, which recognises the error by this text (lowercased: "bind an account for this provider in the armory"),** with its tests; otherwise the "no account bound" error stops being classified |
| MCP tool descriptions, `crates/mcp/src/tool_schemas.rs` (the Global Memory tools: "managed exclusively through the Armory UI", "shown in the Armory Global Memory list", "the Armory UI's own Remove button", "the Armory can bring entries into an isolated channel") | "the Knowledge pane". Agents read these, so they should name what the operator sees |
| Placeholder startup file, `backend/providers.rs` ("use Armory -> Memory -> Global") | "Knowledge → Global" |
| `widgets.json` labels, layout preview summary (`layout_file.rs`) | the new names |

**The bundle file format** keeps its letters and changes its meaning: ABF becomes **Agent
Bundle Format** (§4.6). The save-dialog filter "Armory Bundle" becomes "Agent bundle". Also
unchanged: CSS class names and the sensitive-keyword entry `"armory"` in `reactive/sanitize.rs`.

### 4.5 Tests that pin the Armory

These change with it: `armory-view.test.tsx` (which becomes tests for the two panes and their
section navs), `block-registry.test.ts` (the view list, the `trust` alias),
`pane-tab-model.test.ts` and `tab-presets.test.ts` (fixtures using `defwidget@armory`),
`zoom.test.ts`, `failure-accessory.test.ts`, `AgentStartupModal.test.tsx`,
`claude-translator.test.ts`, and the Rust tests around the error text
(`agents/failure.rs`, `identity/resolver/inject.rs`, `server/tests.rs`, `reactive/tests.rs`),
layouts (`layout/tests.rs`) and `muxspect_handlers.rs`. The UI screenshot script
(`scripts/ui-screenshots/shots.mjs`) opens the "Armory" widget and needs the new names.

### 4.6 Bundles and the Agent Bundle Format

**What a bundle is** (operator, 2026-10-05): a package of an agent's **instructions, MCP
servers, memory and skills**. It isn't tied to a model or a harness, so **a bundle can be bound
to any agent**, whatever CLI it runs (Claude Code, Codex, Gemini, …).

**ABF becomes "Agent Bundle Format".** The acronym, the `.abf` extension and the importer stay;
"Armory" leaves the name.

**Where the code is today** (`frontend/types/rpc/Bundle.ts`, `backend/bundle_export.rs`,
`bundle_import.rs`):

- A bundle row pins `provider` (`claude` | `codex` | `gemini` | empty) and `model`. That's
  what ties a bundle to one harness today.
- ABF v0.2 already made the content harness-aware rather than harness-bound: default
  `instructions` plus optional `instructions_by_provider` variants
  (`SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`), and native
  memory as a component.
- The manifest is `armory.json`, and the schema URLs are `…/schemas/armory-bundle/v0.1|v0.2/…`.

**Proposed:**

1. **A bundle stops pinning a harness.** `provider` and `model` leave the bundle; the agent it's
   bound to decides them. If a bundle needs to say something per harness, it uses the existing
   `instructions_by_provider` variants. Existing rows keep their values as an optional
   "suggested for" hint, shown but not enforced, so no saved bundle changes behaviour on
   upgrade.
2. **Binding to any agent.** At spawn, the bound bundle is composed for the agent's own
   harness: default instructions plus that harness's variant; MCP servers and skills where the
   harness supports them. When a harness can't take a component (say, skills on a CLI without
   a skills mechanism), the spawn says so rather than dropping it silently.
3. **The format's own names.** ABF v0.3 writes the manifest as `bundle.json` and the schema as
   `…/schemas/agent-bundle/v0.3/…`, without `provider`/`model` as required fields. The importer
   keeps reading `armory.json` and the `armory-bundle` v0.1/v0.2 schemas indefinitely, since
   users have those files.

This is a format and binding change, larger than the pane split. It deserves its own spec
before it's built; this section records the direction.

### 4.7 Cutting ABF v0.3

The rename and the harness-agnostic binding ship as a new format version (operator,
2026-10-05). What v0.3 is:

| | v0.2 (today) | v0.3 |
|---|---|---|
| Name | Armory Bundle Format | **Agent Bundle Format** (still ABF, still `.abf`) |
| Manifest | `armory.json` | `bundle.json` |
| `$schema` | `…/schemas/armory-bundle/v0.2/bundle.schema.json` | `…/schemas/agent-bundle/v0.3/bundle.schema.json` |
| Top-level `provider` / `model` | the bundle's harness | gone as required fields; an optional `suggestedFor` hint (§4.6) |
| Components | instructions (+ per-provider variants), context files, MCP servers, skills, native memory | unchanged |
| Requirements file | `requirements.json` (`credentialProvider`) | unchanged, under the new schema path |

**Compatibility:** the importer reads v0.1, v0.2 and v0.3 (both manifest names, all three
schema paths) indefinitely. The exporter writes v0.3 only. A v0.2 bundle imported into this
version becomes harness-agnostic, keeping its `provider` / `model` as the hint.

**Release checklist:**

1. Code (`agentmux`): `bundle_export.rs` writes v0.3; `bundle_import.rs` accepts all three;
   the manifest and schema constants; tests for each version in both directions.
2. Schemas (`agentmux-docs`, `public/schemas/`): publish
   `agent-bundle/v0.3/bundle.schema.json` and `requirements.schema.json`. **Also publish the
   missing `armory-bundle/v0.2/` schemas:** today's exporter already writes a v0.2 `$schema`
   URL, but the docs site hosts only `armory-bundle/v0.1/`, so that link is broken now.
3. Docs: the ABF page as in §5.1, with a "What changed in v0.3" section and the version
   history.
4. Release notes: the changeset for the release, saying ABF v0.3 and the rename.

### 4.8 The default widget bar

The widget bar shows the operator's own pinned list when they've set one (`widget:pinned`,
`frontend/app/window/action-widgets-config.ts`); otherwise it's derived from the defaults in
`crates/srv/src/config/widgets.json` (`display:pinned`, ordered by `display:order`). Today
those defaults pin four: Agent, Swarm, Armory, Sysinfo.

**New default (operator, 2026-10-05):** eleven pinned, in this order; everything else goes in More:

| # | Widget | Key | Today |
|---|---|---|---|
| 1 | Agent | `defwidget@agent` | pinned |
| 2 | **Connectors** | `defwidget@connectors` (new) | replaces Armory |
| 3 | **Knowledge** | `defwidget@knowledge` (new) | replaces Armory |
| 4 | Swarm | `defwidget@swarm` | pinned |
| 5 | Hangar | `defwidget@files` | in More |
| 6 | Terminal | `defwidget@terminal` | in More |
| 7 | Editor | `defwidget@editor` | in More |
| 8 | Browser | `defwidget@browser` | in More |
| 9 | Messengers | `defwidget@messengers` (the group: Discord, Slack, Telegram, WhatsApp, Teams) | in More |
| 10 | Sysinfo | `defwidget@sysinfo` | pinned |
| 11 | Help | `defwidget@help` | in More |

Everything else goes in More: Drone, Warden, Media, Toolchain and Settings.

**How:** set `display:pinned` and give `display:order` 1–11 in that order in `widgets.json`,
then renumber the rest after them. Today several widgets share an order value (Drone and Armory
both 3, Sysinfo and Warden 4, Editor and Hangar 6), so their relative order is accidental; give
each a distinct value. The `armory` widget entry goes (its blocks migrate, §4.3).

**Who sees it:** new installs, and anyone who has never customised the bar, since their bar is
derived from these defaults. Someone with their own `widget:pinned` keeps it, except that an
`armory` entry becomes `connectors` + `knowledge` (§4.3). The pin-widgets menu and More are
unchanged.

**Width:** eleven pinned buttons is far more than today's four. Check the bar at a narrow window: if
it overflows, it should collapse into More the way it already does, not wrap or clip.

Tests: `action-widgets-config.test.ts` (the derived default list and its order), and the
fixtures in `pane-tab-model.test.ts` and `tab-presets.test.ts` that name `defwidget@armory`.

## 5. Phases

Each phase is one PR that leaves the app consistent.

1. **Two panes and entry points.** The `connectors` and `knowledge` pane tabs with their
   section navs, every entry point in §4.2 opening the right pane and section, the §4.3
   migration of saved blocks, layout files and widget pins, and the new default widget bar
   (§4.8). The `armory` view stays registered
   only to migrate old blocks. Frontend text naming the Armory changes here, since what it
   names is gone.
2. **Server and agent-facing text.** §4.4's Rust strings, with the error matcher in the same
   commit, the MCP tool descriptions and the placeholder file.
3. **Clean-up, a release later.** Remove the `armory` view and the `app:identity` alias once
   saved state has had a release to migrate; rename internal identifiers only if it's cheap.

Each phase has a matching docs PR in `agentmux-docs` (§5.1), opened alongside it, so the
site never describes a UI that's gone. ABF v0.3 (§4.7) is its own pair of PRs.

### 5.1 Docs: `agentmux-docs`

The public docs site (`agentmux-docs`, checked at `6888f74`) describes all of this. What
each change touches:

**The Armory split** (about 115 mentions of "Armory" across 20 pages):

| Page | Change |
|---|---|
| `armory.md` (Accounts, Memory → Global/Personal, Bundles, MCP Servers, Skills, Opening the Armory, Portable bundles) | split into a **Connectors** page (Accounts, MCP servers) and a **Knowledge** page (Global, Personal, Skills, Bundles); "Opening the Armory" becomes the entry points of §4.2; keep `/armory/` working as a redirect to them (`astro.config.mjs`) |
| `memory.md` ("Memory bundles") | uses the vocabulary 09-09 retired; rewrite as **Bundles** (instructions, MCP, memory, skills; bind to any agent, §4.6), keeping its native-memory section |
| `pane-types.md` (an Armory entry; the Swarm section) | two pane types instead of one |
| `first-agent.md`, `auth.md`, `identity.md`, `quickstart.md`, `getting-started.md`, `installation.md` | "the Armory → Accounts" becomes "Connectors → Accounts", and so on |
| `multi-instance.md`, `warden.md`, `config.md`, `glossary.md`, `main-menu.md` (the hamburger item), `keybindings.md` (the "Identity & Memory" command) | the new names and entry points; the glossary gains Connectors and Knowledge |
| pages that describe the widget bar: `getting-started.md`, `first-agent.md`, `browser-pane.md`, `identity.md` (and `armory.md`, split above) | the new default buttons (§4.8) |
| `internals/agent-app-api.md` | the Global Memory tool text (§4.4) |
| `security/data-sovereignty.md`, `security/identity-credential-storage.md`, `security/reactive-event-bus.md` | one mention each |

**ABF v0.3** (§4.7): `abf.md` (title, manifest name, `$schema` URL, the `provider` / `model`
section, a v0.3 changes section), `glossary.md` (the ABF entry), and `public/schemas/` (new
v0.3 schemas, and the missing v0.2 ones now).

**Already shipped, docs behind:**

- **Swarm Groups removed, Stats added** (#4330): `subagent-watcher.md` still documents the
  **Groups ▾** button and saved groups; replace it with the **Stats** button and panel (what
  each count means). `pane-types.md`'s Swarm section gains a line on Stats.
- **Wheel edge skid** (#4335): not documented. A short note in `pane-types.md`'s Agent section:
  at a preview's edge the first few wheel notches are held, with a line on that edge, before
  the pane scrolls; trackpad swipes are held until they pause.

The docs repo is public, takes a changeset on every PR, and its own `CLAUDE.md` covers the
rest. The two "already shipped" items can go now, independent of this spec.

### 5.2 Phase 1 as built

- **One component, two manifests.** `frontend/app/view/section-pane/section-pane.tsx` is the
  sectioned pane (rail, narrow-width tab bar, Ctrl+Wheel zoom, every section mounted and
  hidden); `view/connectors/connectors.tsx` and `view/knowledge/knowledge.tsx` list their
  sections. The Armory's stylesheet moved with it and keeps its class names.
  `section-pane/panes.ts` holds the ids and meta keys, `openConnectors(section)` /
  `openKnowledge(section)`, and the Armory mapping.
- **Titles** read "Connectors · Accounts", "Knowledge · Skills": a section name alone
  ("Global") didn't say which pane it was. Section icons match the Stash's: key and plug; globe, brain,
  wand and layer-group (Bundles keeps its accent).
- **Landing on a section.** `openOrFocusPaneByView` takes a third argument, meta to merge into a
  pane it focuses, so a link to Knowledge → Skills switches an open Knowledge pane to Skills.
  The Connectors and Knowledge widgets focus an open pane of their type instead of opening
  another (§1.2's third problem).
- **Migration.** The `armory` manifest (alias `trust`) is now only a shim: its `create()`
  writes the new view and section into the block's meta, and the block remounts as that
  pane. `layout_file.rs` writes `connectors`/`knowledge` with `config.section` and reads
  `armory` the same way; a block saved as `armory` and never reopened is exported as its new
  pane. Both sides share one mapping, kept in step by tests on each.
- **Palette.** `app:connectors` and `app:knowledge`, found by "accounts", "mcp", "memory",
  "skills", "bundles" through a new `keywords` field; `app:identity` is registered hidden
  and opens Connectors. The macOS app menu lists both.
- **Screenshots.** The `armory-bundles` shot in `scripts/ui-screenshots/shots.mjs` is now
  `knowledge-bundles`.

## 6. Risks

| Risk | Mitigation |
|---|---|
| Muscle memory: the pinned "Armory" button moves | one-time; the two new buttons sit where it was |
| A saved Armory block or layout opens blank | §4.3 rewrites by saved section; tests for each legacy value, including `trust` and `native_memory` |
| The "no account bound" error stops being recognised | change the message and `failure.rs`'s matcher together, with the existing tests |
| Agents' instructions mention "the Armory" (MCP descriptions, Global Memory entries, agent CLAUDE.md files) | Phase 2 updates the descriptions AgentMux owns; operator-written text is theirs to change, and the hidden `app:identity` alias covers old habits for a release |
| Someone looks for "Accounts" as a place of its own | Accounts is the first, default section of Connectors, and the palette finds Connectors by "Accounts" |
| "Knowledge" suggests retrieval (RAG) more than configuration | the sections inside are labelled plainly; revisit only if users are confused |

## 7. What this does not change

The managers themselves, how accounts, memory, skills, MCP servers or bundles are stored, the
`Memory*` and `GlobalMemory*` agent tools, the bundle file format's acronym and extension,
per-agent surfaces (the Stash, the `identity` and `memory` pane tabs that sit with an agent),
and the status bar's MuxBus cloud sign-in.

## 8. Effort

Roughly: Phase 1 is a medium frontend PR, mostly composition, entry points and migration,
plus a small `layout_file.rs` change. Phase 2 is small but touches error classification, so
it gets its own review. Phase 3 is trivial.

## 9. Decisions

**Decided (operator, 2026-10-05):**

1. **Two panes: Connectors and Knowledge.**
2. **Connectors** has two sections: **Accounts** (sign-ins to providers and services) and **MCP
   servers**. Connectors aren't only MCP; the Armory's Accounts list is already a list of
   connectors.
3. **Knowledge** has four: **Global**, **Personal**, **Skills** and **Bundles**.
4. **Bundles** package instructions, MCP, memory and skills, are harness-agnostic and bind to
   any agent; **ABF becomes Agent Bundle Format, cut as v0.3** (§4.6, §4.7).
5. **The default widget bar:** Agent, Connectors, Knowledge, Swarm, Hangar, Terminal, Editor,
   Browser, Messengers, Sysinfo, Help; the rest in More (§4.8).

**Decided (operator, 2026-10-05, taking the recommendations):**

6. **Knowledge opens on Global,** what most visits are for.
7. **The 09-09 Phase 4 fold** (Global as a filter inside Bundles) can follow later, inside
   Knowledge.
8. **ABF v0.3 gets its own spec** (§4.6–4.7 written up) before it's built.
9. **Docs for what's already shipped** (Swarm Stats replacing Groups, the wheel skid) are fixed
   now, in the first `agentmux-docs` PR.

**Open, not proposed here:**

10. **Should Connectors → Accounts also take MuxBus cloud sign-in** from the status bar popover?
    AgentMux Cloud is already an account tile there, so the popover would become a shortcut to
    it. Not proposed here.
