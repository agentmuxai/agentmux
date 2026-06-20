# Memory & Identity Architecture — Simplified Design

**Date:** 2026-06-19  
**Status:** Proposal  
**Author:** Design review after PR #1581 (Trust Center accounts UI) + PRs #1584–#1588 (agent pane modals + native memory backend)

---

## 1. Problem Statement

Two pairs of concepts share names but mean different things, causing confusion at every layer:

| Word | Trust Center meaning | Agent Pane meaning |
|---|---|---|
| **Memory** | `db_memory_bundles` — config presets (provider + model + instructions + tools) | Native memory files `~/.claude/projects/.../memory/*.md` (Claude's learned facts) |
| **Identity** | `db_identity_accounts` — credential library (API keys, OAuth tokens) | Per-agent keychain — all the credentials this agent needs to operate |

The result: two different surfaces (Trust Center modal, agent pane header icons) that deal with overlapping concepts through different models, different UI patterns, and different naming.

---

## 2. Current State — What Exists

### 2.1 Layer 1: Trust Center (Global Library)

Opened via the hamburger menu → "Identity & Memory" (the `BundleManagerModal`).

**Accounts tab** (`AccountsTab` → `IdentityViewModel`)
- Manages `db_identity_accounts` rows
- Each row = a named credential: API key, OAuth token, PAT, env ref, role
- Provider-scoped: one account = one provider (anthropic, github, aws, etc.)
- CRUD: create / edit / delete / validate / reauth
- Status tracking: valid / expired / invalid / checking / unknown

**Memory tab** (`MemoryManager` → `MemoryViewModel`)
- Manages `db_memory_bundles` rows
- Each row = a named "agent config preset": provider CLI choice, model, system instructions, context files, MCP servers, skills
- `is_global = true` → content is concatenated into every agent's `CLAUDE.md` at launch
- `is_blank = true` → the "vanilla CLI" singleton (no provider, no instructions)
- CRUD: create / edit / delete

**Not yet surfaced in the UI:**
- `IdentityBundle` + `IdentityBinding` types exist in `gotypes.d.ts` (v7 schema, `db_identity_bundles` table) — named groupings of accounts ("Parko's keys" = { anthropic: key-1, github: key-2 })
- These are schema-level additions that have not been connected to any UI yet

### 2.2 Layer 2: Agent Pane (Per-Agent Operational View)

**ID card icon** → `AgentIdentityModalPanel` (via `ModalLayer`)
- Shows the agent's **keychain** — the full set of credentials it will use at runtime
- An agent is bound to a single AI provider at creation (e.g. "claude"), but it may need credentials for many services: the AI API key, GitHub, Slack, AWS, etc. The keychain is where all of those live.
- Each row in the keychain = one service + one named account from the Trust Center library
- `AgentAccounts = Partial<Record<AccountProvider, string | null>>` — JSON blob stored on `AgentDefinition.accounts` (legacy) or via `db_agent_identity_links` junction table (v6 normalized form)

**Brain icon** → `AgentMemoryModalPanel` (via `ModalLayer`)
- Phase 1 placeholder: shows the memory folder path only
- Points at `~/.claude/projects/<sanitized-working-dir>/memory/`
- Backend RPCs already implemented (Phase 2 complete): `agent:memory:list`, `agent:memory:read_file`, `agent:memory:write_file` in `native_memory_handlers.rs`
- Phase 3 (not started): full file browser + editor UI

### 2.3 What Gets Injected at Agent Launch

```
Agent launch sequence:
  1. Collect global Memory bundles (is_global=true FROM db_memory_bundles)
     → Concatenate instructions into CLAUDE.md in agent's working directory
  
  2. Resolve per-agent account assignments (db_agent_identity_links / AgentDefinition.accounts)
     → Inject credentials as env vars / keychain lookups before process spawn
  
  3. Claude Code session starts
     → MEMORY.md auto-loaded from ~/.claude/projects/<sanitized>/memory/
     → Claude may write new facts autonomously during the session
```

---

## 3. What's Broken

### 3.1 The "Memory" name collision

`db_memory_bundles` ("Memory" in the Trust Center) = configuration presets — provider, model, system prompt, tools. These are human-authored and define HOW the agent works.

Native memory (`~/.claude/projects/*/memory/`) = facts Claude discovers autonomously — codebase patterns, user preferences, session insights. Claude writes these; humans review and prune them.

Both are called "Memory". The brain icon was chosen specifically for native memory (Claude's brain), but the Trust Center tab also uses the brain icon (`viewIcon = "brain"` in `MemoryViewModel`). Searching for "memory" in the codebase hits both concepts interchangeably.

### 3.2 Identity Bundles are a dangling abstraction

`IdentityBundle` (`db_identity_bundles`) groups accounts into named profiles ("Parko's keys"). This is a v7 schema-level concept. But:
- No Trust Center UI surfaces it
- The agent pane ID card modal assigns accounts directly per-provider (bypassing bundles entirely)
- `AgentDefinitionIdentity` / `db_agent_identity_links` are per-provider direct refs, not bundle refs
- Result: the bundle table exists and can be created, but nothing reads it at agent launch

### 3.3 Two account assignment mechanisms in flight

Legacy path: `AgentDefinition.accounts` (JSON blob, deprecated since v6, comment says "use db_agent_identity_links instead"). New path: `db_agent_identity_links` junction table. The `AgentIdentityModalPanel` still writes the deprecated JSON blob via `UpdateAgentDefinitionCommand.accounts`. Migration is incomplete.

### 3.4 Phase 3 gap

The native memory backend RPCs are complete. The UI is a placeholder. Users can't view or edit native memory files from inside AgentMux today.

---

## 4. Proposed Simplified Architecture

### 4.1 Core principle: rename to separate the concepts

The user-facing name is **Presets** (decided 2026-06-20). "Bundle" was the
original proposal but is ambiguous in the live taxonomy — Identities are also
"bundles" (`IdentityBundle`), and the rail already names tabs by their domain
("Identities", not "Identity Bundles"). "Presets" is a distinct domain noun for
"reusable agent config." The internal type stays `Memory` / `db_memory_bundles`
(backend names unchanged); only user-facing strings change.

| Surface | New user-facing name | Notes |
|---|---|---|
| Memory tab (Trust Center rail) | **Presets** | rail: Accounts · Identities · Brain · Presets |
| Per-item label | **Preset** | "+ New Preset", "Edit Preset" |
| Memory pane (`view: "memory"`) | **Presets** summary | `viewType = "memory"` string stays (persisted block key) |
| `viewIcon` (was `brain`) | `sliders` / `cubes` | reserve the brain icon for native memory only |

**Internal symbols stay** (`MemoryViewModel`, `MemoryDraft`, `memory-*.tsx`,
`memory-view-*` CSS, `db_memory_bundles`, `bundle_memory_*`, `listmemories`).
Renaming the persisted `view: "memory"` key or the RPC/table names is pure churn
with a migration/compat cost and no user benefit; the backend already keeps the
`Memory` type name, so the frontend class staying `MemoryViewModel` is consistent.

**Result:** "brain" = one thing only (Claude's autonomous native memory);
"Presets" = provider-agnostic config. No ambiguity.

### 4.1a Presets are provider-agnostic (decided 2026-06-20)

A preset is the agent's reusable, **provider-agnostic** capability pack:
instructions + context files + MCP servers + skills. It does NOT carry a
provider or model.

- **Provider + model belong to the agent.** `AgentDefinition.provider` is the
  CLI (claude/codex/gemini); the model rides in `AgentDefinition.provider_flags`
  as `--model <x>`. The agent is the single source of truth for what runs.
- The `Memory` bundle's `provider` and `model` fields are **vestigial** — stored
  and editable today, but never read at launch (only `instructions`, context,
  MCP, and skills are consumed). They are a duplicate of provider that does
  nothing.

**Action (cosmetic + cleanup, low risk — the fields are vestigial):**
- Remove the **Provider** and **Model** fields from the Presets editor
  (`memory-manager.tsx`) and from `MemoryDraft` / the wire.
- Leave the `db_memory_bundles.provider` / `.model` columns in place,
  deprecated (written empty) — no migration, no launch path touched.
- A preset can then be paired with an agent of any provider.

### 4.2 Resolve or commit to Identity Bundles

**Option A (recommended): Remove IdentityBundle as a separate layer**

The direct per-provider account assignment (what the ID card modal does today) already solves the problem Identity Bundles were meant to solve. Having both creates complexity without benefit right now.

- Keep: `db_identity_accounts` (the credential library)
- Keep: `db_agent_identity_links` (per-agent per-provider account refs)
- Remove: `db_identity_bundles` + `IdentityBinding` from the active architecture (keep schema rows if they exist but don't expose them in UI)
- Clarify: The Trust Center "Accounts" tab is the library. The agent pane ID card is per-agent assignment. Done.

**Option B: Surface Identity Bundles in the Trust Center**

If the team wants named identity profiles ("Parko's keys"), add an "Identities" tab to the Trust Center that manages `db_identity_bundles` + `IdentityBinding` rows. The agent pane ID card then picks a bundle rather than individual accounts. This is a cleaner UX for users with many agents using the same credential set, but adds UI complexity.

### 4.3 Complete the legacy account migration

The `AgentIdentityModalPanel` still writes `AgentDefinition.accounts` (the deprecated JSON blob). To fully adopt the v6 path:
- Wire `AgentIdentityModalPanel` to write `db_agent_identity_links` via a new `UpdateAgentIdentityLinksCommand` RPC instead of `UpdateAgentDefinitionCommand.accounts`
- Keep backward-compat read: on agent launch, merge both the junction table and the JSON blob (junction wins on conflict)
- Eventually drop the JSON blob

This is a backend + frontend change. Worth a dedicated PR.

### 4.4 Build Phase 3: native memory browser

The backend is done. The UI gap is the only thing preventing the brain icon from being useful. The spec (`SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §5`) describes the full layout. Prioritize this because it's the visible payoff for the native memory backend work already shipped.

---

## 5. Target Architecture (simplified)

```
┌─────────────────────────────────────────────────────────────────────────┐
│  TRUST CENTER (global library — hamburger → "Trust Center")             │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  Accounts tab                                                   │   │
│  │  Library of named credentials (API keys, OAuth, PATs, roles)   │   │
│  │  Provider-scoped. Stateful (valid / expired / invalid).         │   │
│  │  CRUD + validate + reauth.                                      │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │  Bundles tab  (renamed from "Memory")                           │   │
│  │  Named agent config presets: provider + model + instructions    │   │
│  │  + context files + MCP servers + skills                         │   │
│  │  is_global=true → injected into ALL agents at launch (CLAUDE.md)│   │
│  │  is_blank=true  → "vanilla CLI" singleton                       │   │
│  │  CRUD.                                                          │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘

                    ↓ at agent launch: global bundles → CLAUDE.md
                    ↓ at agent launch: account refs → env vars / keychain

┌─────────────────────────────────────────────────────────────────────────┐
│  AGENT PANE HEADER ICONS (per-agent operational view)                   │
│                                                                         │
│  [🪪 ID card]  Keychain modal                                           │
│    The agent's keychain — all credentials it needs to operate.          │
│    Agent provider is fixed at creation; this covers every service it   │
│    uses: AI API key, GitHub, Slack, AWS, etc. Picks named accounts     │
│    from the Trust Center library.                                       │
│    Writes to: db_agent_identity_links (v6 normalized form)              │
│                                                                         │
│  [🧠 Brain]   Native Memory modal                                        │
│    Claude's autonomous fact store for this agent.                       │
│    Path: ~/.claude/projects/<sanitized-working-dir>/memory/             │
│    Files: MEMORY.md (index) + topic files (user/project/feedback/ref)  │
│    Phase 3: full browser + editor. Edit → NativeMemoryWriteFile RPC.   │
└─────────────────────────────────────────────────────────────────────────┘

                    ↓ Claude writes autonomously during sessions
                    ↓ MEMORY.md auto-loaded at session start
                    ↓ Topic files loaded on-demand by Claude

┌─────────────────────────────────────────────────────────────────────────┐
│  CLAUDE SESSION (runtime)                                               │
│                                                                         │
│  CLAUDE.md         — rules + policies (from global bundles)             │
│  memory/MEMORY.md  — fact index (Claude-written, auto-loaded)           │
│  memory/*.md       — topic files (Claude-written, on-demand)            │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Layer separation invariant

| What | Written by | Stored in | Injected as | Visible in UI |
|---|---|---|---|---|
| Rules / policies / tool configs | Human (via Bundles tab) | `db_memory_bundles` | `CLAUDE.md` at every launch | Trust Center → Bundles tab |
| Credentials | Human (via Accounts tab) | `db_identity_accounts` | Env vars / keychain at spawn | Trust Center → Accounts tab |
| Agent keychain (per-agent credential set) | Human (via ID card / Keychain modal) | `db_agent_identity_links` | Same as credentials | Agent pane → Keychain modal |
| Discovered facts / patterns | Claude autonomously | `~/.claude/projects/*/memory/` | `MEMORY.md` at session start | Agent pane → Brain modal (Phase 3) |

**The invariant:** A Memory Bundle must never contain a fact Claude discovered, and a native memory file must never contain a rule a human intended to enforce. If that drift happens, a quarterly review should promote facts to bundles or prune them.

---

## 6. What to Build (Prioritized)

### Priority 1 — Phase 3: Native memory browser (high user value, backend already done)

**Files to create/modify:**
- `frontend/app/view/agent/agent-native-memory-model.ts` — `AgentNativeMemoryModel` class (reactive file list, select/read/edit/save/create)
- `frontend/app/view/agent/components/AgentNativeMemoryModal.tsx` — replace the placeholder with the real two-column browser + editor
- `frontend/app/view/agent/agent-native-memory.scss` — styles

**Layout:**
```
┌──────────────────────────────────────────────────────────────────┐
│  Memory — <agent name>                                   [✕]    │
│  ~/.claude/projects/<sanitized-path>/memory/                     │
├────────────────┬─────────────────────────────────────────────────┤
│ MEMORY.md  [i] │  <content or edit textarea>                     │
│ user_profile   │                                                  │
│ proj_agentmux  │                                                  │
│                │                          [Edit]  [Save] [Cancel]│
│ + New file     │                                                  │
└────────────────┴─────────────────────────────────────────────────┘
```

Empty state: "No memory files yet. Claude creates this folder when it first saves a fact. [+ Create MEMORY.md]"

### Priority 2 — Rename Memory → Presets + strip provider/model

**Scope:** user-facing rename + removal of two vestigial fields. No schema
migration. See §4.1 and §4.1a for the decisions.

User-facing rename (strings only — internal symbols stay, see §4.1):
- Trust Center rail label: "Memories" → **"Presets"**
- `memory-manager.tsx` strings: "+ New Memory" → "+ New Preset", "Edit Memory" /
  "New Memory" → "Edit Preset" / "New Preset", "Select a Memory bundle…" →
  "Select a preset…", confirm/empty/error copy → "preset"
- `memory-model.ts`: `viewText` / `viewName` "Memory" → "Presets";
  `viewIcon` `"brain"` → `"sliders"` (reserve brain for native memory)
- `memory-view.tsx` + `bundle-summary.tsx`: standalone pane label "Memory" →
  "Presets" (extend `BundleSummaryPanel` kind to render "Presets")
- `CLAUDE.md` "Not widgets" table: update the "Memory" row

Provider-agnostic cleanup (§4.1a):
- Remove the **Provider** and **Model** form fields + the `provider` / `model`
  keys from `MemoryDraft` and `draftToWire`
- Leave `db_memory_bundles.provider` / `.model` columns deprecated (written
  empty); the launch path never read them, so nothing breaks

Backend: no changes needed — the table and RPC names stay (`db_memory_bundles`,
`listmemories`, etc.); only UI strings and the two form fields change.

### Priority 3 — Migrate identity assignment to v6 junction table

**Scope:** backend + frontend, medium risk.

- New RPC: `UpdateAgentIdentityLinksCommand` → replaces writing `AgentDefinition.accounts` JSON blob
- `AgentIdentityModalPanel.handleUpdate`: call new RPC instead of `UpdateAgentDefinitionCommand`
- Agent launch: read `db_agent_identity_links` primary, fall back to `AgentDefinition.accounts` for legacy rows
- Mark `AgentDefinition.accounts` field deprecated in the type comment with a removal target version

### Priority 4 — Decide and implement Identity Bundles (Option A or B from §4.2)

Defer until Priority 1–3 are done. The schema exists; the architectural decision (remove vs surface) should be made after the team has shipped the native memory browser and seen how users interact with it.

---

## 7. Files and Symbols Reference

| Concept | Frontend | Backend |
|---|---|---|
| Account library | `identity-model.ts`, `identity-view.tsx`, `AccountsTab` | `db_identity_accounts`, `account.*` RPCs |
| Bundle library | `memory-model.ts`, `memory-manager.tsx`, `MemoryManager` | `db_memory_bundles`, `bundle.*` RPCs |
| Per-agent account assignment | `AgentIdentityModalPanel.tsx`, `AgentIdentityPanel`, `AgentAccounts` | `db_agent_identity_links` (v6) / `AgentDef.accounts` (legacy) |
| Native memory | `AgentMemoryModalPanel.tsx` (placeholder), `AgentNativeMemoryModel` (TODO) | `native_memory_handlers.rs` — list/read/write |
| Global bundle injection | — | `agent_config.rs` `build_settings_with_hooks()`, reads `bundle_memory_list_global()` |
| Memory path computation | `previewMemoryPath()` in AgentMemoryModal | `memory_dir_for_cwd()` in native_memory_handlers.rs |
