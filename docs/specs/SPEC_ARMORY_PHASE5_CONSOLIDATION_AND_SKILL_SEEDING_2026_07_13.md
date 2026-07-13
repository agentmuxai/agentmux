# SPEC — Armory Phase 5: drop Identities, rename/reorder tabs, seed a starter Skill catalog

**Status:** Draft — spec only, no code written yet (per explicit request).
**Type:** Design + Phase 5 (Bundle-as-container v2, the Policy primitive, and this Phase 5
consolidation are the three items issue #2024 lists as remaining after Phase 4a-4c shipped
2026-07-13 — see `SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`).
**Verify before acting:** all file:line citations below checked against `main` @ `18afb8ab`
(post v0.53.3 release) on 2026-07-13. Re-verify if this doc is read more than a few days later.

---

## 0. Scope — four changes, one pane

1. Remove the Armory "Identities" tab.
2. Rename Armory's "Memory" tab label to "Memories".
3. Reorder Armory's tabs to: **Accounts, Memories, Skills, MCP Servers, Bundles**.
4. Seed the (currently empty) global Skills catalog with a starter set, adapted from
   `a5af/claw`'s `templates/skills/*.md`.

Items 1-3 are one mechanical change to `frontend/app/view/armory/armory-view.tsx`'s `RAIL`
array. Item 4 is a separate backend seeding mechanism. Both are independent and can ship as
separate PRs.

---

## 1. Remove the Identities tab

### 1.1 Current state

`frontend/app/view/armory/armory-view.tsx:16-23`, the `RAIL` array:

```ts
const RAIL: { id: ArmorySection; label: string; icon: string }[] = [
    { id: "accounts",   label: "Accounts",   icon: "key" },
    { id: "identities", label: "Identities", icon: "id-card" },
    { id: "brain",      label: "Memory",     icon: "brain" },
    { id: "memories",   label: "Bundles",    icon: "layer-group" },
    { id: "mcp",        label: "MCP Servers", icon: "plug" },
    { id: "skills",     label: "Skills",      icon: "wand-magic-sparkles" },
];
```

This is already the documented target direction, not a new idea: `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md:76` states the target has *no* Identities tab — "fold it into the Accounts + Bundle surfaces" — and `PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md:211` says the same ("Account is the primitive; 'identity' is a derived view"). Phase 3 (PR-C, #2061) deliberately deferred the actual removal, keeping a read-only `AgentIdentitiesPanel` "for one release." This spec is that deferred removal.

### 1.2 What breaks — this is NOT a no-loss removal

`AgentIdentitiesPanel` (`frontend/app/view/identity/agent-identities-panel.tsx`) is currently
**the only functioning UI in the app** that shows, per agent, which accounts it actually
launches with (agent picker → Provider/Account/Status table, backed by
`RpcApi.ListAllAgentIdentitiesCommand` joined against `db_agent_identity_links`).

The natural-looking replacement — the agent-pane's own **"Identity" tab** (`view: "identity"`,
distinct from Armory's tab; CLAUDE.md already calls this out as the intended per-agent surface)
— does **not currently work**. It renders `BundleSummaryPanel kind="Identity"`
(`frontend/app/view/identity/identity-pane-view.tsx:27`), whose own code comment
(`frontend/app/view/bundle-summary.tsx:18-25`) documents an explicit **DATA GAP**: it can't
resolve "this agent uses Identity: X" because the launched identity id lives on the
`AgentInstance` row, not reachable from this settings block. It degrades to a pointer-only
stub — body text + a button that calls `openOrFocusPaneByView("armory")` with **no section
param**, so it always lands on Armory's default `"accounts"` tab today, not even a working deep
link to the current Identities tab.

### 1.3 Decision: fix the agent-pane Identity tab's data gap, don't just delete

Rather than remove Identities and accept the loss (option 1 from the investigation) or bolt a
new "used by" affordance onto `AccountsGallery` (option 3 — real, unscoped design work), this
spec proposes **option 2**: close the documented data gap on the agent-pane Identity tab so it
shows *that specific agent's* linked account(s) directly. This is the architecturally correct
destination anyway (CLAUDE.md already treats the agent-pane Identity tab as the intended
per-agent surface, and the Armory Identities tab was always meant to be temporary), and per the
investigation the missing piece is narrow: `ListAllAgentIdentitiesCommand` already returns the
join data — the gap is plumbing the *current* agent's id into `BundleSummaryPanel`/
`identity-pane-view.tsx`'s call site, not new backend work.

**Concrete change:**
- `frontend/app/view/identity/identity-pane-view.tsx`: replace the generic
  `<BundleSummaryPanel kind="Identity" />` stub with a small panel that calls
  `ListAllAgentIdentitiesCommand`, filters to the current block's `agentId`, and renders the
  same Provider/Account/Status row shape `AgentIdentitiesPanel` already renders for one agent
  (extract/share that row-rendering into a small component both can use, rather than
  duplicating markup).
- Delete `frontend/app/view/identity/agent-identities-panel.tsx`, its Armory RAIL entry, the
  `"identities"` member of `ArmorySection` (`frontend/app/view/armory/armory-model.ts:4`), and
  the corresponding `<div class="bundle-manager-pane bundle-manager-pane--identity">` block in
  `armory-view.tsx` (lines 60-62 today).
- `bundle-summary.tsx`'s "manage" button (currently a bare `openOrFocusPaneByView("armory")`,
  landing on `"accounts"`) can stay pointed at Armory's Accounts tab — that's now the correct
  target for *managing* an identity/account link, per the same fold-into-Accounts direction.

**Out of scope for this spec:** the shared row-rendering component's exact shape, and whether
`BundleSummaryPanel` needs a more general "resolve current agent id" hook reusable by future
per-agent settings tabs. Flag as a design decision for whoever implements — worth a quick look
at whether `BundleSummaryPanel`'s other `kind`s (if any) have the same gap.

### 1.4 Test coverage to add

- Agent-pane Identity tab renders the correct account(s) for the block's own `agentId`, not
  another agent's.
- Armory pane no longer has an "Identities" entry (rail item count, no `AgentIdentitiesPanel`
  import).
- `ArmorySection` type no longer accepts `"identities"` (compile-time; no runtime test needed).

---

## 2. Rename "Memory" → "Memories"

### 2.1 Change

`frontend/app/view/armory/armory-view.tsx:19`:
```diff
-    { id: "brain", label: "Memory", icon: "brain" },
+    { id: "brain", label: "Memories", icon: "brain" },
```

Confirmed genuine inconsistency, not a considered choice: every other Armory tab label is
plural (Accounts, Bundles, MCP Servers, Skills — Identities too, before removal), and the
2026-07-02 rename that settled "Memory" over "Brain" (#2025) only decided the *term*, never
addressed pluralization. The team's own architecture doc
(`docs/specs/archive/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md:45`) already lists
Armory's contents as "Accounts, **Memories**, MCP Servers, Skills, Briefs, and Bundles" —
consistent plural, confirming this was always the intended terminology.

### 2.2 Companion surface — decide, don't silently skip

`frontend/app/view/agent/components/AgentSetupModal.tsx:53` has its own, **independent** tab
list with `{ id: "memory", label: "Memory" }` (mounting `AgentNativeMemoryModal`, the per-agent
native-memory browser — a different component from Armory's global `GlobalBrainManager`, but
the same underlying "native memory" concept). Nothing in the code forces this to change in
lockstep with Armory's rename, but leaving it singular while Armory says "Memories" is the same
kind of inconsistency this spec is fixing. **Recommend renaming both** for terminology
consistency across the two "Memory" surfaces — implementer's call if there's a reason to keep
them different (there doesn't appear to be one).

### 2.3 Stale doc reference, not code — fix while touching the area

`agentmux-srv/src/config/widgets.json:213`, the `defwidget@armory` widget's `description`
field, still reads `"Manage accounts, identities, brain, and presets"` — stale on three counts
(identities being removed per §1, "brain" vs. the shipped "Memory"/"Memories" label, and
"presets" being the pre-rename name for Bundles). Cosmetic (widget-bar tooltip only) but cheap
to fix in the same PR: `"Manage accounts, memories, skills, MCP servers, and bundles"`.

---

## 3. Reorder tabs: Accounts, Memories, Skills, MCP Servers, Bundles

### 3.1 Change

Confirmed safe — no persisted "last active tab," no index-dependent logic anywhere under
`frontend/app/view/armory/` (`section()` is a plain component-local Solid signal, resets to
`"accounts"` on every mount; routing is entirely by `id` string, never array position).

`RAIL` becomes (after §1's removal and §2's rename):

```ts
const RAIL: { id: ArmorySection; label: string; icon: string }[] = [
    { id: "accounts", label: "Accounts",    icon: "key" },
    { id: "brain",    label: "Memories",    icon: "brain" },
    { id: "skills",   label: "Skills",      icon: "wand-magic-sparkles" },
    { id: "mcp",      label: "MCP Servers", icon: "plug" },
    { id: "memories", label: "Bundles",     icon: "layer-group" },
];
```

Note the `id`s stay exactly as they are today (`"brain"` still means the Memories/native-memory
tab, `"memories"` still means the Bundles tab — those id strings predate this rename and
renaming them too would be pure churn with no user-visible benefit; only array **position** and
the one **label** string change).

For readability, also reorder the matching `<div class="bundle-manager-pane">` mount blocks
(armory-view.tsx lines 57-74) to the same order — not required for correctness (they're keyed
by `section() !== id`, not position) but keeps the file's visual order matching the UI.

`ArmorySection`'s union member order in `armory-model.ts:4` is cosmetic-only (TS doesn't care)
— reorder to match for the same readability reason, not required.

### 3.2 AgentSetupModal.tsx — out of scope

Confirmed fully independent tab list/component, no "Identities" concept, and its own tab order
(`Accounts, Memory, MCP Servers, Skills`) isn't required to match Armory's. No change needed
here for the reorder itself — only the optional Memories rename from §2.2 touches this file.

---

## 4. Seed the global Skills catalog

### 4.1 Current state — ships empty

- A "Skill" is 9 plain SQL columns (`db_skills`: `id, name, trigger, skill_type, description,
  content, is_global, created_at, updated_at` — `agentmux-srv/src/backend/storage/
  migrations.rs:410-422`), **not** a markdown+frontmatter file. It's materialized to
  `.claude/commands/<trigger>.md` only at agent-launch config-gen time
  (`agentmux-srv/src/backend/agent_config.rs:100-106`) — the DB row is the source of truth.
- No seed mechanism exists for `db_skills` today. The only existing seed path
  (`agentmux-srv/src/backend/agent_seed.rs`, `SEED_MANIFEST`) writes to the **legacy**
  `db_agent_skills` table (one per-agent-template skill each, e.g. Claude's "Startup
  Verification"), not the v1 standalone catalog. `skill_list_global()` — what the Armory Skills
  tab and every agent's launch-time global-skill injection both read — queries
  `db_skills WHERE is_global = 1`, which is empty on every fresh install today.
- Global skills need **no per-agent bind step** to take effect: `write_agent_config_files`
  (`agentmux-srv/src/server/app_api/agent_open.rs:550-573`) unions every agent's own
  `db_agent_skills_ref` rows with **all** `is_global = 1` rows automatically. So "pre-populate
  a starter set" is exactly: insert `is_global = 1` rows into `db_skills` once. No fan-out,
  no per-agent wiring.

### 4.2 Source material — grounded in real, verified, popular public skills

**Revised 2026-07-13 (second revision), superseding both prior drafts of this section.**
`a5af/claw`'s skills were ruled out as source material — private, project-specific content not
to be committed into AgentMux. The first revision proposed writing purely original content
instead; the user then asked to ground the starter set in what's *actually popular* publicly,
not just invented. Researched via the `deep-research` skill (its synthesis step degraded to a
placeholder, but the underlying search/fetch pipeline surfaced real leads) and verified
directly against GitHub:

| Source | Stars (verified 2026-07-13) | License | Relevance |
|---|---|---|---|
| [`obra/superpowers`](https://github.com/obra/superpowers) | **253,678** | MIT | The single most popular known Claude Code skill collection — an entire "agentic skills framework & software development methodology." Contains `systematic-debugging`, `test-driven-development`, `requesting-code-review`, `receiving-code-review`, `finishing-a-development-branch`, `using-git-worktrees`, `verification-before-completion` — almost exactly the category set this spec wants, already well-known and battle-tested at real scale. |
| [`anthropics/skills`](https://github.com/anthropics/skills) | 160,825 | — | Anthropic's own official Agent Skills repo + spec/template. Mostly creative/document-tooling skills (docx/pptx/xlsx/design) rather than general engineering discipline — used here to confirm the official skill-authoring shape, not as direct content source. |
| [`anthropics/claude-code-security-review`](https://github.com/anthropics/claude-code-security-review) | 5,532 | — | Anthropic's own official AI-powered security review GitHub Action. Best available source for the security-review skill specifically. |
| [`awesome-skills/code-review-skill`](https://github.com/awesome-skills/code-review-skill) | 1,379 | — | Popular community code-review skill (framework-specific: React/Vue/Rust/TS) — corroborates code review as a high-demand category, not used as direct content (too framework-coupled for a generic seed). |

AgentMux's Skill schema (`db_skills`: name/trigger/description/content, `skill_type` default
`"prompt"` — confirmed in §4.1) is a single freeform-text instruction block, not a multi-file
package with bundled scripts the way `anthropics/skills`' own examples ship (e.g.
`webapp-testing` includes actual Python helper scripts + a LICENSE file). So even under
`superpowers`' permissive MIT license, the right move is **adapted, single-block content
citing the real source**, not a verbatim multi-file import — matching AgentMux's schema shape
while keeping the well-known name/framing that makes each skill recognizable.

**Starter set (6 skills), each grounded in a real, verified, popular source:**

| Skill | Trigger | Grounded in | Focus |
|---|---|---|---|
| Systematic Debugging | `systematic-debugging` | `obra/superpowers/skills/systematic-debugging` | Root cause before fixes — "NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST." Reproduce, isolate, verify the fix actually addresses the root cause, not just the symptom. |
| Test-Driven Development | `tdd` | `obra/superpowers/skills/test-driven-development` | Write the failing test first, cover edge cases and failure paths not just the happy path, keep tests fast/deterministic. |
| Code Review (Requesting & Receiving) | `code-review` | `obra/superpowers/skills/requesting-code-review` + `receiving-code-review` | Review early and often; what a reviewer should check (correctness, tests, scope creep, security-sensitive diffs); how to respond to review feedback without defensiveness. |
| Git Commit & Branch Hygiene | `commit-hygiene` | `obra/superpowers/skills/finishing-a-development-branch` + `using-git-worktrees` | Atomic commits, messages that say why not just what, clean branch completion, when worktrees beat branch-switching. |
| Verification Before Completion | `verification-before-completion` | `obra/superpowers/skills/verification-before-completion` | Don't declare a task done on "it should work" — actually run it, check the output, confirm the specific thing that was asked for happened. |
| Security Review Basics | `security-basics` | `anthropics/claude-code-security-review` | Input validation, secrets handling, common injection classes, least-privilege defaults — the checks an AI-assisted security review action looks for. |

Small and curated — six skills, each traceable to a specific, real, popular, verifiable source,
not a bulk import or an invented list.

### 4.3 Implementation shape

Mirror the existing `widgets.json`/`tool-catalog.json` config-file pattern
(`agentmux-srv/src/config/`) rather than a Rust-embedded manifest like `agent-seed.json`'s
`SEED_MANIFEST` (that one seeds per-agent-template legacy skills; this is a one-time global
catalog seed, a different shape):

1. New `agentmux-srv/src/config/starter-skills.json` — an array of `{name, trigger, skill_type,
   description, content}` objects (no `id`/`is_global`/timestamps — those are assigned at
   insert time), content authored per §4.2's table.
2. A one-time seed step — **not** a schema migration (migrations run unconditionally on every
   startup across every install; seeding should be idempotent and skippable if the user already
   has global skills, including ones they've deleted on purpose). Model this after how
   `agent_seed.rs` gates its own seeding, or gate on "only seed if `db_skills WHERE is_global=1`
   is empty AND this is a fresh install" (check for an existing "fresh install" signal elsewhere
   in the codebase — e.g. how the default channel/first-run state is detected — rather than
   inventing a new one).
3. Insert via the same `skill_upsert_unique_global`-family path the `skill.catalog.upsert` RPC
   handler already uses (`agentmux-srv/src/backend/storage/skills.rs:475` region) — reuse
   existing validated insert logic, don't hand-roll new SQL.

**Open question for whoever implements:** should this seed run on every fresh install
unconditionally, or be opt-in (e.g. a "load starter skills" button in the empty-state Armory
Skills tab)? Unconditional is simpler and matches "pre-populate," but an unconditional silent
DB write on first launch is a bigger behavioral change than a button a user clicks — flagging
as a real product decision, not assuming unconditional is correct by default.

### 4.4 Test coverage to add

- Seed step is idempotent (running twice doesn't duplicate rows).
- Seed step doesn't overwrite/duplicate if the user already has global skills (including after
  deleting a seeded one — don't resurrect it).
- Each seeded skill's `content` renders sensibly through the existing config-gen path
  (`.claude/commands/<trigger>.md` materialization) — a smoke test, not full agent launch.

---

## 5. Suggested PR split

1. **PR A** — §1 (remove Identities, fix the agent-pane Identity-tab data gap) + §2 (Memories
   rename, both surfaces) + §3 (reorder) + the `widgets.json` description fix. One cohesive
   frontend-plus-small-backend-plumbing change (the data-gap fix needs the existing
   `ListAllAgentIdentitiesCommand` RPC, no new backend surface).
2. **PR B** — §4 (skill seeding). Independent, backend-only, no dependency on PR A. Answer the
   §4.3 unconditional-vs-opt-in question before starting.

---

## 6. Sources

- `docs/specs/SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`
- `docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md`
- `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`
- `specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md`
- `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`
- `docs/specs/archive/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md`
- GitHub issue #2024 (Armory pane consolidation tracker)
- `a5af/claw` (`templates/skills/*.md`), read 2026-07-13
