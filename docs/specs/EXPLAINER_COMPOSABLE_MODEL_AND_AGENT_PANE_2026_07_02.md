# Explainer: The Composable Agent Model & the Agent Pane's Two Icons

**Date:** 2026-07-02
**Author:** Agent1
**Purpose:** Explain the composable agent model (what replaces "preset") in plain terms, connect it to what a user actually touches, and record the **agent-pane header decision**: replace the two icons (brain/Memory + id-card/Identity) with a **single `id-card` icon** opening a unified per-agent management modal (Accounts · Memory · MCP · Skills · Briefs · Bundle). See §4.
**Companion docs:** `specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md` (the merged decision), `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` (the implementation phases), `docs/specs/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` (the umbrella rename).

---

## 1. The old model: one opaque "preset"

Today an agent's setup is smeared across three stores with a confusing shape:

- **"Preset"** — an inline grab-bag: instructions + context files + MCP servers + skills, all embedded in one record (stored, misleadingly, in `db_memory_bundles`).
- **Identity** — per-provider account assignment, stored separately (`db_identity_bundles` → bindings → `db_identity_accounts`).
- **Memory ("the brain")** — native memory, separate again.

Three problems: you can't define an MCP server or a skill **once** and reuse it (they're buried inside a preset and copies drift); there's no single object that answers "what is this agent's full setup"; and the storage names collide ("bundle" means two different things).

## 2. The new model: reusable primitives + one optional collection

The merged proposal (product-owner decision) **retires "preset"** and separates the reusable building blocks from the thing that assembles them. There are **six named primitives**, each first-class, independently managed, and shareable:

| Primitive | What it is | Loads… |
|-----------|-----------|--------|
| **Account** | The provider login + credential the agent runs as (OAuth/key, per-account config dir). "Identity" is just a *derived view* over an agent's bound accounts — **not** a stored object. | at pane open (env injection: `GH_TOKEN`, `ANTHROPIC_API_KEY`, …) |
| **Memory** | Persistent learned knowledge (the native memory store). "The brain" is a nickname, not the canonical term. | recalled at startup |
| **MCP Server** | An external tool/connection surface (URL/stdio + which tools). Broken out of preset. | connects at startup; tools on demand |
| **Skill** | An on-demand instruction/knowledge module (a folder), loaded when invoked. Broken out of preset. Complements — does **not** replace — the standing instructions in `CLAUDE.md` (see the standing-instructions note below). | only when invoked |
| **Brief** | **The first message** injected when a pane opens — the startup/kickoff payload. It is *additive to* `CLAUDE.md`, not a replacement for standing instructions. | as the first user message |
| **Bundle** | The optional **named collection** — references (not copies of) the primitives above, so you can apply a whole set in one step and share it. Replaces "preset". | resolves to its referenced primitives |

> **Standing instructions stay in `CLAUDE.md` (product decision, 2026-07-02).** The Claude CLI natively auto-loads `CLAUDE.md` from the working dir as its always-on project instructions; AgentMux assembles it from `soul` + `agentmd` + `memory` + skills index (`agent_config.rs:28`). We are **retaining** it — an earlier proposal draft (§3.4) floated retiring it, but that would break standing-instruction delivery for Claude agents. So `soul`/`agentmd` stay in `CLAUDE.md`; Brief and Skills are additive, not replacements. (Other CLIs get their own native file — `AGENTS.md`, `GEMINI.md`, etc.)

**The key mental shift:** an agent binds primitives **directly** (any number of MCP servers + skills, one Brief, ≤1 Account per provider, a Memory). A **Bundle** is *sugar* — a saved, shareable set of those bindings — **not** a required wrapper. Effective config = direct bindings ∪ included Bundles, with a direct binding overriding the same item from a Bundle.

"Reference, don't copy": edit a primitive once and every agent using it updates. Provider/model + workspace stay on the **agent** (they're not portable the way credentials/skills are).

## 3. Where you touch it: the Armory vs. the Agent Pane

There are two surfaces, and they play different roles:

- **The Armory** (formerly "Trust Center") — the **library**. Where you *define and manage* primitives across all agents: Accounts, Memories, MCP Servers, Skills, Briefs, and Bundles. Global vs. agent-owned sharing lives here. Reached from the hamburger (≡) → Armory.
- **The Agent Pane** — the **per-agent quick access**. Its title-bar icons let you jump straight to *this agent's* slice of the model without opening the whole Armory.

So: Armory = manage the whole catalog; Agent pane icon = "open THIS agent's setup."

## 4. The agent pane icon — today vs. the decision

**Today** the agent pane's title bar renders **two** icon buttons (`frontend/app/view/agent/agent-model.ts:141`, `endIconButtons`):

| Icon (today) | Title (today) | Opens | Primitive |
|--------------|---------------|-------|-----------|
| `brain` | "Agent memory" | Memory modal | **Memory** |
| `id-card` | "Agent identity" | Identity modal (`IdentityTab = "accounts" \| "assignments"`) | **Account** |

That second modal is *already* account management — its tabs are literally **Accounts** and **Assignments** (`frontend/app/view/identity/identity-model.ts:22`). It's only *labeled* "identity."

### DECISION (product owner, 2026-07-02): consolidate to a **single icon**

Replace the two buttons with **one** `id-card` icon that opens a **unified agent-management modal** — the agent's own view over *all* its bound primitives: **Accounts, Memory, MCP Servers, Skills, Briefs** (and its **Bundle**, if any). No separate brain/id-card split; one entry point, "see all your stuff for this agent."

- **Icon:** `id-card` (kept — it reads as "who/what this agent is and carries"). Not `brain`, not `key`.
- **Title:** e.g. "Agent setup" / "Manage agent" (not "identity" — the modal is broader than identity now).
- **Modal:** a tabbed surface = the Armory scoped to this agent. Tabs: **Accounts · Memory · MCP · Skills · Briefs · Bundle**. Reuses the Armory's primitive managers, filtered to this agent's bindings, with "add from library / create new" deep-linking to the full Armory.

Rationale: with five+ primitives now first-class, a per-primitive icon each would crowd the title bar (§5's rejected option). One icon → one modal keeps the header clean and gives the user a single "what is this agent made of" view — which is exactly the "one-stop shop" the Bundle concept describes, surfaced per-agent.

**Net:** the pane header goes from `brain` + `id-card` → a single `id-card` "Agent setup" button. The Memory modal and the Accounts (née Identity) modal both fold into the unified modal as tabs.

## 5. Why one icon, not two or six

Options considered, once MCP/Skills/Brief are first-class:

1. ~~**Keep two** (Memory + Accounts)~~ — still splits related config across two buttons and omits MCP/Skills/Brief.
2. ~~**Surface all primitives**~~ — five/six icons crowd the title bar; that density belongs in the Armory, not a pane header.
3. **One icon → unified modal** — **chosen.** A single `id-card` button opens the agent's full setup (all primitives as tabs). Clean header, complete picture, and it mirrors the Bundle "one-stop shop" idea at the per-agent level. Everything remains reachable; nothing is hidden — it's just consolidated behind one affordance instead of scattered across two.

Because primitives bind directly, the modal simply lists what this agent is bound to, tab by tab; the Armory remains the place to define/share primitives across agents.

## 6. How this maps to the refactor phases

From `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`:

- **Phase 1 (shipped):** MCP Servers + Skills became first-class primitives (`mcp.*`/`skill.*` App API).
- **Phase 2 (in flight):** "Preset" → "Bundle" at the UI + App API layer.
- **Phase 3 (needs product decisions):** collapse the identity-bundle layer → **Account-direct**; "Identity" becomes a derived view; the resolver moves to `instance/bundle → account`. **This is the phase that consolidates the pane header to a single `id-card` "Agent setup" button opening the unified management modal (§4 decision)** — the Memory and Accounts modals fold in as tabs alongside MCP / Skills / Briefs / Bundle.
- **Phase 4:** storage rename (`db_memory_bundles → db_bundles`, `db_identity_accounts → db_accounts`, `db_identity_bundles` removed) — the honest end-state where "bundle" means exactly one thing.

So the single-icon **Agent setup** pane is the **Phase 3 UI outcome**: the pane already opens the account surface (as "identity"); Phase 3 unifies it with Memory (and the other primitives) behind one modal and makes the data model match. Implementation note for Phase 3: in `agent-model.ts` `endIconButtons`, replace the two-button array with a single `id-card` button whose `click` opens the unified modal; retire `_openMemoryModal` / `_openIdentityModal` in favor of one `_openAgentSetupModal` (or reuse the Armory's tabbed manager scoped to the agent).

## 7. Product decisions (proposal §9)

**Resolved (2026-07-02):**
1. ✅ **Derived-Identity UX** — the pane consolidates to one `id-card` "Agent setup" icon (§4); "Identities" folds in as the **Accounts** tab; identity stays a derived view.
3. ✅ **Static `memory` blob & 4. `soul`/`agentmd`** — **`CLAUDE.md` is RETAINED.** The Claude CLI natively loads it as standing instructions; `soul`/`agentmd`/`memory` stay in `CLAUDE.md`. Brief (first message) and Skills (on-demand) are *additive*, not replacements. This reverses the proposal's §3.4 "retire CLAUDE.md" stance.

**Still open:**
2. **Policy primitive** — hooks + `.claude/settings.json` permissions: a distinct 7th primitive, or folded in? (Affects whether the pane/Armory grows another surface.)

The Policy question is the only remaining gate on Phase 3's full shape; the pane's transition to the single **Agent setup** icon can proceed regardless.
