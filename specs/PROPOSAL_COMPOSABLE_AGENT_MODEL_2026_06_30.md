# Proposal: A Composable Agent Model for the Trust Center

**Date:** 2026-06-30
**Status:** Proposal — for discussion (naming + IA decisions needed)
**Author:** AgentX
**Driver:** "I'd like a cleaner model… break out skills and MCP into the Trust Center… should we call them bundles? preset bundles? would a preset be a one-stop shop?"
**Related:** CLAUDE.md "Not widgets" (Identity / Presets); `db_memory_bundles`, `db_identity_bundles`; the per-agent identity provisioning work (`SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md`); the agent App API identity/preset/memory handlers.

---

## 1. The problem

Today "**preset**" is an opaque, inline grab-bag — it bundles *instructions +
context files + MCP servers + skills* in one record, while **identity** and
**memory (the brain)** are separate concepts living elsewhere. Three pains follow:

1. **"Preset" is overloaded and inline.** MCP servers and skills are buried *inside*
   a preset, so you can't define an MCP server or a skill **once** and reuse it
   across agents — you re-embed it per preset, and copies drift.
2. **The grouping is incomplete and implicit.** What a user actually wants to
   reason about — *"who is this agent (identity), what does it know (brain/skills),
   what can it reach (MCP), how does it behave (instructions)"* — is split across
   "preset" + "identity" + "memory" with no single coherent object.
3. **Naming debt.** Presets are stored in `db_memory_bundles` (but a preset is
   **not** the brain), and identities in `db_identity_bundles` — two different
   "bundles." Adding "preset bundle" on top would compound the collision.

The fix is not a bigger preset. It's to **separate the reusable building blocks
from the thing that assembles them**, and give each a clear home in the Trust
Center.

## 2. Design principles

- **Primitives are first-class and reusable.** Define an MCP server, a skill, an
  identity, a brain, an instruction set **once**; reference it from many agents.
- **One assembly object is the "one-stop shop."** A single named thing answers
  "what is this agent's full setup" — by **referencing** primitives, not inlining
  them.
- **Reference, don't copy.** The assembly stores pointers; edit a primitive once
  and every agent using it updates.
- **The Trust Center is the trust + capability surface.** Identity (credentials),
  MCP (external reach), and skills (injected instructions) are all *trust
  decisions* — they belong together where you grant and review capability.
- **Names should not collide.** Avoid reusing "bundle" for a new concept (already
  means identity bundle / the legacy `db_memory_bundles`).

## 3. Proposed model — primitives + one assembly

### 3.1 The five primitives (Trust Center, each independently managed & shareable)

| Primitive | What it is | Today | Proposed home |
|---|---|---|---|
| **Identity** | Account + credential the agent authenticates as (OAuth/key, per-account `CLAUDE_CONFIG_DIR`) | Agent-pane tab; `db_identity_bundles` | Trust Center › **Identities** |
| **Brain** (memory) | Persistent learned knowledge (native memory store) | "the brain", per-identity | Trust Center › **Brains** |
| **MCP server** | An external tool/connection surface (URL/stdio + which tools) | inlined in preset | Trust Center › **MCP Servers** (break out) |
| **Skill** | On-demand instruction/knowledge module (a folder) | inlined in preset | Trust Center › **Skills** (break out) |
| **Brief** (instructions/context) | Standing system-level instructions + context files (the CLAUDE.md-equivalent) | the rest of "preset" | Trust Center › **Briefs** |

> Each primitive carries its own **ownership/sharing** (agent-owned vs **global**,
> mirroring the App API's existing global-preset guard) and its own audit trail.
> MCP servers and identities especially are security-sensitive — first-class
> status gives them proper review, not burial inside a preset blob.

### 3.2 The assembly — the "one-stop shop"

One named object answers *"what is this agent's full setup"* by **referencing** a
selection of primitives:

```
Assembly "AgentX-standard"
  ├─ identity:   → Identities/AgentX
  ├─ brain:      → Brains/AgentX
  ├─ mcp:        → [MCP/github, MCP/agentmux, MCP/aws]
  ├─ skills:     → [Skills/git-workflow, Skills/reagent, …]
  └─ brief:      → Briefs/host-agent
```

- It holds **references**, not copies (principle: reference, don't copy).
- Apply it to an agent in one step → the agent inherits all five.
- An **Agent** = an Assembly **+** provider/model **+** workspace. (Provider/model
  and workspace stay on the agent — they're not portable across machines/providers
  the way the assembly is. This preserves the current "provider/model belong to the
  agent" rule.)

**Yes — the assembly is the one-stop shop.** But it's a *thin* assembler over
independently-managed primitives, not a monolith. That's the difference from
today's inline preset.

## 4. Naming (the crux of the question)

Two things need names: **the assembly** and **the broken-out primitives**.

### 4.1 The assembly — candidates

| Name | For | Against |
|---|---|---|
| **Profile** *(recommended)* | "Agent profile" naturally groups identity + memory + capabilities; familiar (browser profiles, **AWS_PROFILE** — which claw already uses per agent); no collision | one more new word vs. keeping "preset" |
| **Preset** (redefine) | incumbent; "your agent's preset setup" | currently means the *narrow* inline thing; expanding it to include identity (credentials) + brain stretches "preset"; still stored in `db_memory_bundles` |
| **Loadout** | evocative of "equipped capabilities" | identity/brain aren't quite "loadout items"; informal |
| **Bundle / Preset bundle** | matches the user's instinct | **collides** with identity bundles + `db_memory_bundles`; "preset bundle" stacks two overloaded terms — **not recommended** |

**Recommendation: rename the assembly to `Profile`** and retire "preset" (or keep
"preset" as a deprecated alias for one release). A Profile = identity + brain +
MCP + skills + brief. If you prefer minimal churn, the fallback is **keep "Preset"
as the assembly** but redefine it as reference-based and add identity + brain — I'd
still avoid "bundle" either way.

### 4.2 The primitives — naming

Keep them concrete and singular: **Identity, Brain, MCP Server, Skill, Brief**.
("Brain" is already in use for memory; "Brief" is a crisp word for the standing
instructions + context that "instructions" or "rules" undersell.) Open to
"Instructions" instead of "Brief" if you prefer the literal term.

### 4.3 Why NOT "bundle"
"Bundle" is already two things (`db_identity_bundles`, `db_memory_bundles`).
Reusing it for the assembly would mean three meanings of "bundle." Pick a fresh
word for the assembly and reserve "bundle" for nothing new.

## 5. Trust Center information architecture

```
Trust Center
├─ Profiles        ← the one-stop assemblies (apply to agents)
├─ Identities      ← accounts / credentials (per-account CLAUDE_CONFIG_DIR)
├─ Brains          ← memory stores
├─ MCP Servers     ← external tool/connection surfaces  (NEW: broken out)
├─ Skills          ← instruction/knowledge modules       (NEW: broken out)
└─ Briefs          ← standing instructions + context files
```

A **Profile** view shows the five reference slots and lets you pick from the
primitive lists; "create new" from within a slot deep-links to the primitive
editor. Agents pick a Profile + model + workspace.

## 6. Trust & sharing model

- Each primitive is **agent-owned** or **global** (shared). The App API already
  guards global presets (`preset.upsert` rejects modifying `is_global`); generalize
  that guard to **every** primitive: an agent may write its own, never another's or
  a global one without authorization. (Ties directly to the S1/S4 work in the agent
  App API.)
- **MCP** and **Identity** carry the heaviest trust weight (external reach +
  credentials). First-class status means they get explicit grant/review in the
  Trust Center instead of riding hidden inside a preset.
- A Profile referencing a global primitive can't silently fork it; it points at the
  shared definition.

## 7. Backend / migration sketch

- **Reference model:** Profiles store primitive **IDs**, not inline JSON. Migrate
  today's inline presets by extracting their MCP/skills into standalone
  `mcp_servers` / `skills` rows and rewriting the preset as references → a `Profile`.
- **Naming debt:** plan to rename `db_memory_bundles` → `db_profiles` (or keep the
  table, rename the concept) so "memory bundle" stops meaning "preset." Keep
  `db_identity_bundles` as **Identities** (or rename to `db_identities`). This is a
  storage migration; do it behind the App API so agents see only the new surface.
- **Compatibility:** keep "preset" as a read alias for one release; new writes go to
  Profiles.
- **App API:** add `mcp.*` and `skill.*` agent commands paralleling the existing
  `preset.*` / `memory.*`, with the same S1 + ownership/global guards. The
  `preset.*` surface becomes `profile.*`.

## 8. What this unlocks (incl. the claw import)

The `a5af/claw` "AgentX/AgentY" scheme maps **cleanly** onto this model — it stops
being an awkward "dump it in memory" and becomes:
- **Brief:** the claw `CLAUDE.md` + startup instructions
- **Skills:** the claw `templates/skills/*` (each a first-class Skill)
- **MCP Servers:** the claw `.mcp.json` entries (each a first-class MCP server)
- **Identity:** AgentX's account (the per-agent identity work)
- **Brain:** AgentX's accumulated memory
- → assembled into a **Profile "AgentX"**, applied to the AgentX agent (and shared
  with AgentY where identical).

That's the "clean model" the import wanted: a reusable, shareable, reviewable
loadout — not config smeared across three stores.

## 9. Open decisions (need product-owner input)

1. **Assembly name:** `Profile` (recommended) vs. redefine `Preset` vs. other.
   *Not* "bundle."
2. **"Brief" vs "Instructions"** for the standing-instructions primitive.
3. **Scope of v1:** break out **MCP + Skills** first (the user's explicit ask), or
   land the whole five-primitive + Profile IA at once?
4. **Identity in the assembly?** Reference it from the Profile (recommended), or
   keep identity selected separately at launch (status quo)?
5. **Rename `db_memory_bundles`** now (clean) or defer the storage migration and
   only rename the concept/UI first?

## 10. Recommendation

Adopt the **primitives + Profile** model. Name the assembly **Profile**; break out
**MCP Servers** and **Skills** as first-class Trust Center primitives first (the
highest-value, lowest-risk slice and the user's explicit ask), then fold
identity + brain references into the Profile. Avoid "bundle" for any new concept.
Sequence: (1) break out MCP + Skills, (2) introduce Profile as a reference-based
rename of Preset, (3) wire identity + brain references, (4) backend rename +
migration.
