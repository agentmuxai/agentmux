# Explainer: The Composable Agent Model & the Agent Pane's Two Icons

**Date:** 2026-07-02
**Author:** Agent1
**Purpose:** Explain the composable agent model (what replaces "preset") in plain terms, and answer a specific question: *the agent pane's two title-bar icons — are they related to this?* **Yes.** This document connects the model to what a user actually touches.
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
| **Skill** | An on-demand instruction/knowledge module (a folder). Broken out of preset. **All instructional/behavioral content lives here** — there is no always-on instruction blob. | only when invoked |
| **Brief** | **The first message** injected when a pane opens — the startup/kickoff payload. That is *all* it is; not standing instructions. | as the first user message |
| **Bundle** | The optional **named collection** — references (not copies of) the primitives above, so you can apply a whole set in one step and share it. Replaces "preset". | resolves to its referenced primitives |

**The key mental shift:** an agent binds primitives **directly** (any number of MCP servers + skills, one Brief, ≤1 Account per provider, a Memory). A **Bundle** is *sugar* — a saved, shareable set of those bindings — **not** a required wrapper. Effective config = direct bindings ∪ included Bundles, with a direct binding overriding the same item from a Bundle.

"Reference, don't copy": edit a primitive once and every agent using it updates. Provider/model + workspace stay on the **agent** (they're not portable the way credentials/skills are).

## 3. Where you touch it: the Armory vs. the Agent Pane

There are two surfaces, and they play different roles:

- **The Armory** (formerly "Trust Center") — the **library**. Where you *define and manage* primitives across all agents: Accounts, Memories, MCP Servers, Skills, Briefs, and Bundles. Global vs. agent-owned sharing lives here. Reached from the hamburger (≡) → Armory.
- **The Agent Pane** — the **per-agent quick access**. Its title-bar icons let you jump straight to *this agent's* slice of the model without opening the whole Armory.

So: Armory = manage the whole catalog; Agent pane icons = "what is THIS agent using, right now."

## 4. The agent pane's two icons — the direct answer

The agent pane's title bar renders exactly two icon buttons (`frontend/app/view/agent/agent-model.ts:141`, `endIconButtons`):

| Icon (today) | Title (today) | Opens | Primitive |
|--------------|---------------|-------|-----------|
| `brain` | "Agent memory" | Memory modal | **Memory** |
| `id-card` | "Agent identity" | Identity modal (`IdentityTab = "accounts" \| "assignments"`) | **Account** |

That second modal is *already* account management — its tabs are literally **Accounts** and **Assignments** (`frontend/app/view/identity/identity-model.ts:22`). It's only *labeled* "identity."

**So yes — the two icons are Memory and Accounts, and it is the same refactor.** Under the composable model:
- "Identity" is not a primitive; **Account** is. The `id-card` "Agent identity" button becomes the **Accounts** button (icon `id-card` → `key`, title "Agent identity" → "Accounts"), opening the account-assignment surface it already is.
- The `brain` "Agent memory" button stays as **Memory** (the model keeps "Memory" as canonical; "brain" is just the nickname).

That gives the two-icon pane you described: **Memory + Accounts** — the two per-agent primitives you touch most, surfaced right on the pane; the rest (MCP Servers, Skills, Briefs, and Bundles) live in the Armory, reachable when you want to compose or share.

## 5. Should the pane show more than two icons?

An open design question (not decided here). Options, once MCP/Skills/Brief are first-class:

1. **Keep two** (Memory + Accounts) — the highest-frequency per-agent touchpoints; everything else via the Armory or the agent's Bundle. Least clutter. *(Recommended default.)*
2. **Add a Bundle button** — one more icon that opens "this agent's Bundle" (its full reference set: accounts + memory + MCP + skills + brief in one view). This matches "a Bundle is the one-stop shop when you want one."
3. **Surface all primitives** — five/six icons. Powerful but crowds the title bar; better suited to the Armory.

The model doesn't force a choice — because primitives bind directly, the pane can surface as few or as many as makes sense. The natural fit is **Memory + Accounts on the pane** (your two icons), with an optional **Bundle** affordance for "show me this agent's whole loadout."

## 6. How this maps to the refactor phases

From `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`:

- **Phase 1 (shipped):** MCP Servers + Skills became first-class primitives (`mcp.*`/`skill.*` App API).
- **Phase 2 (in flight):** "Preset" → "Bundle" at the UI + App API layer.
- **Phase 3 (needs product decisions):** collapse the identity-bundle layer → **Account-direct**; "Identity" becomes a derived view; the resolver moves to `instance/bundle → account`. **This is the phase that turns the pane's `id-card` "identity" button into the `key` "Accounts" button.**
- **Phase 4:** storage rename (`db_memory_bundles → db_bundles`, `db_identity_accounts → db_accounts`, `db_identity_bundles` removed) — the honest end-state where "bundle" means exactly one thing.

So the "Memory + Accounts" pane you're picturing is the **Phase 3 UI outcome**: the pane already opens the account surface; Phase 3 renames it from "Identity" to "Accounts" and makes the data model match.

## 7. Open product decisions this depends on (proposal §9)

Phase 3 (hence the icon relabel) is gated on:
1. **Derived-Identity UX** — drop the "Identities" tab entirely, or keep a read-only derived view for one release?
2. **Policy primitive** — hooks + `.claude/settings.json` permissions: a distinct 7th primitive, or folded in? (Affects whether the pane/Armory grows another surface.)
3. **Static `memory` blob** (in today's CLAUDE.md) → merge into Brief vs. native Memory.
4. **`soul`/`agentmd` → Skills** — retire the always-on CLAUDE.md instruction blob (proposal's stance) as part of this, or separately.

Answering these unblocks the pane's transition to the clean **Memory + Accounts** (+ optional Bundle) surface.
