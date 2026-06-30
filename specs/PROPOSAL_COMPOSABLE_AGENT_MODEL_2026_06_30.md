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
- **The Trust Center is the trust + capability surface.** Accounts (credentials),
  MCP (external reach), and skills (injected instructions) are all *trust
  decisions* — they belong together where you grant and review capability.
- **Primitives bind directly; collections are optional.** An agent can bind
  primitives ad-hoc; a named **collection** is sugar for reuse, not a required
  wrapper.
- **Names should not collide, and shed legacy.** Don't reuse "bundle" for any new
  concept, and drop the misleading `_bundles` suffix on the storage names.

## 3. Proposed model — primitives + an optional collection

### 3.1 The primitives (Trust Center, each independently managed & shareable)

| Primitive | What it is | Today | Proposed home |
|---|---|---|---|
| **Account** | The provider login + credential the agent authenticates as (OAuth/key, per-account `CLAUDE_CONFIG_DIR`) | `db_identity_accounts`, wrapped in a redundant identity-**bundle** layer | Trust Center › **Accounts** (drop the bundle wrapper — §3.3) |
| **Brain** (memory) | Persistent learned knowledge (native memory store) | "the brain", per-account | Trust Center › **Brains** |
| **MCP server** | An external tool/connection surface (URL/stdio + which tools) | inlined in preset | Trust Center › **MCP Servers** (break out) |
| **Skill** | On-demand instruction/knowledge module (a folder) | inlined in preset | Trust Center › **Skills** (break out) |
| **Brief** (instructions/context) | Standing system-level instructions + context files (the CLAUDE.md-equivalent) | the rest of "preset" | Trust Center › **Briefs** |

> Each primitive carries its own **ownership/sharing** (agent-owned vs **global**,
> mirroring the App API's existing global-preset guard) and its own audit trail.
> Accounts and MCP servers especially are security-sensitive — first-class status
> gives them proper review, not burial inside a preset blob.

### 3.2 Direct binding is the base; an Assembly is just a named collection

The base mechanism is **direct binding**: an agent binds the primitives it needs
(any number of MCP servers + skills, one Brief, ≤1 Account per provider, a Brain).
An **Assembly** is **not a required wrapper** — it's a *named, reusable collection*
of those same bindings, for "apply this whole set in one step" and "share it across
agents." It's convenience + reuse, not a new layer the agent must have.

```
Assembly "AgentX"            (a saved collection — optional)
  ├─ accounts: [Accounts/AgentX-claude, Accounts/AgentX-github, Accounts/AgentX-aws]
  ├─ brain:    → Brains/AgentX
  ├─ mcp:      → [MCP/github, MCP/agentmux, MCP/aws]
  ├─ skills:   → [Skills/git-workflow, Skills/reagent, …]
  └─ brief:    → Briefs/host-agent
```

- It holds **references**, not copies (principle: reference, don't copy).
- **An Agent's effective config = its direct bindings ∪ the Assemblies it
  includes.** Precedence: a direct binding overrides the same item from an Assembly
  (so an Assembly is a baseline you can locally tweak).
- Provider/model and workspace stay on the **agent** (not portable across
  machines/providers the way the bound primitives are) — preserving the current
  "provider/model belong to the agent" rule.

**Yes — an Assembly is the one-stop shop when you want one.** But because primitives
bind directly, you can also run an agent with no Assembly at all (ad-hoc bindings),
or mix an Assembly with a couple of direct overrides. That flexibility is the
difference from today's mandatory, inline preset.

### 3.3 Do we still need "Identity"? No — Account is the primitive

Identity today is **three layers**: `bundle → bindings → accounts`, where a
**bundle is itself a collection** — exactly one account *per provider*
(claude→A, github→B, aws→C). That is *literally an assembly of accounts*. Once the
Assembly concept exists, the identity-bundle layer is **redundant**:

- The real primitive is the **Account** (provider + credential). Drop the
  identity-**bundle** object entirely.
- Agents bind **accounts directly**. The only invariant the bundle used to enforce
  — **≤1 account per provider** — moves to **resolution time** (the resolver picks
  the single account per provider from the agent's direct bindings + assemblies;
  a conflict is a validation error, surfaced like any other).
- A named, reusable set of accounts is just an **Assembly** (which can hold accounts
  alongside MCP/skills/brief).
- "**Identity**" survives only as a *user-facing descriptor* — "what is this agent
  running as" — a derived view over the agent's bound accounts, **not a stored
  object**.

This also **simplifies the in-flight provisioning spec**
(`SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md`): an Account already carries
its own `OAuthConfigDir`, so there is no bundle to provision — log in → get an
Account → bind it. The resolver change (instance → account directly, instead of
instance → bundle → binding → account) is the one real refactor; flag it there.

## 4. Naming (the crux of the question)

Two things need names: **the assembly** and **the broken-out primitives**.

### 4.1 The collection — candidates

| Name | For | Against |
|---|---|---|
| **Assembly** *(recommended — product owner leaning here)* | Literally means "a collection assembled from parts" — accurate for a named set of bound primitives; no collision; reads well ("the AgentX assembly") | newish word in this domain (minor) |
| **Profile** | "Agent profile" groups capabilities; familiar (browser profiles, **AWS_PROFILE** — claw uses it per agent) | connotes a *persona* more than a *collection*; less precise now that it's an optional collection, not a mandatory wrapper |
| **Preset** (redefine) | incumbent | currently the *narrow* inline thing; stretching it to a reference-based collection muddies the rename |
| **Bundle / Preset bundle** | matches an early instinct | **collides** — "bundle" already means two things; a third meaning compounds the debt — **not recommended** |

**Recommendation: name the collection an `Assembly`.** It precisely denotes "a
collection" (which is what it is, now that primitives bind directly and the
Assembly is optional), and it collides with nothing. Retire "preset" (keep as a
read alias for one release).

### 4.2 The primitives — naming

Keep them concrete and singular: **Account, Brain, MCP Server, Skill, Brief**.
("Brain" is already in use for memory; "Brief" is a crisp word for the standing
instructions + context that "instructions" or "rules" undersell.) Open to
"Instructions" instead of "Brief" if you prefer the literal term. Note: **there is
no "Identity" primitive** — Account is the primitive; "identity" is a derived view
(§3.3).

### 4.3 Why NOT "bundle"
"Bundle" is already two things (`db_identity_bundles`, `db_memory_bundles`).
Reusing it for the assembly would mean three meanings of "bundle." Pick a fresh
word for the assembly and reserve "bundle" for nothing new.

## 5. Trust Center information architecture

```
Trust Center
├─ Assemblies      ← named collections you apply to agents (optional, reusable)
├─ Accounts        ← provider logins / credentials (per-account CLAUDE_CONFIG_DIR)
├─ Brains          ← memory stores
├─ MCP Servers     ← external tool/connection surfaces  (NEW: broken out)
├─ Skills          ← instruction/knowledge modules       (NEW: broken out)
└─ Briefs          ← standing instructions + context files
```

No **Identities** tab — Account is the primitive; "identity" is a derived view
(§3.3). An **Assembly** view shows reference slots and lets you pick from the
primitive lists; "create new" from a slot deep-links to the primitive editor. An
agent binds primitives directly and/or includes Assemblies, plus picks model +
workspace.

## 6. Trust & sharing model

- Each primitive is **agent-owned** or **global** (shared). The App API already
  guards global presets (`preset.upsert` rejects modifying `is_global`); generalize
  that guard to **every** primitive: an agent may write its own, never another's or
  a global one without authorization. (Ties directly to the S1/S4 work in the agent
  App API.)
- **Accounts** and **MCP servers** carry the heaviest trust weight (credentials +
  external reach). First-class status means they get explicit grant/review in the
  Trust Center instead of riding hidden inside a preset.
- An Assembly referencing a global primitive can't silently fork it; it points at
  the shared definition.

## 7. Backend / migration sketch

- **Reference model:** Assemblies store primitive **IDs**, not inline JSON. Migrate
  today's inline presets by extracting their MCP/skills into standalone
  `mcp_servers` / `skills` rows and rewriting the preset as references → an
  `Assembly`.
- **Drop the `_bundles` suffix — it's legacy and misleading:**
  - `db_memory_bundles` → `db_assemblies` (it's presets, *not* memory, and not a
    "bundle" — doubly wrong today).
  - `db_identity_bundles` → **removed**: the identity-bundle layer collapses (§3.3);
    agents/assemblies reference accounts directly.
  - `db_identity_accounts` → `db_accounts`.
  - Phase it: rename the **concept/UI now** (cheap), do the **storage migration**
    behind the App API later so agents only ever see the clean surface.
- **Resolver change:** spawn resolution moves from `instance → identity_bundle →
  binding → account` to `instance/assembly → account` directly (one account per
  provider, enforced at resolve). Reconcile with
  `SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md`.
- **Compatibility:** keep "preset" / `db_memory_bundles` as a read alias for one
  release; new writes go to Assemblies.
- **App API:** add `mcp.*`, `skill.*`, and `account.*` agent commands paralleling
  the existing `memory.*`, with the same S1 + ownership/global guards. `preset.*`
  becomes `assembly.*`.

## 8. What this unlocks (incl. the claw import)

The `a5af/claw` "AgentX/AgentY" scheme maps **cleanly** onto this model — it stops
being an awkward "dump it in memory" and becomes:
- **Brief:** the claw `CLAUDE.md` + startup instructions
- **Skills:** the claw `templates/skills/*` (each a first-class Skill)
- **MCP Servers:** the claw `.mcp.json` entries (each a first-class MCP server)
- **Account:** AgentX's Claude login (the per-agent provisioning work)
- **Brain:** AgentX's accumulated memory
- → collected into an **Assembly "AgentX"**, applied to the AgentX agent (and shared
  with AgentY where identical).

That's the "clean model" the import wanted: reusable, shareable, reviewable
building blocks — not config smeared across three stores.

## 9. Open decisions (need product-owner input)

1. **Collection name:** `Assembly` (recommended; product owner leaning here) vs.
   `Profile`. *Not* "bundle."
2. **"Brief" vs "Instructions"** for the standing-instructions primitive.
3. **Scope of v1:** break out **MCP + Skills** first (the explicit ask), or land the
   whole primitives + Assembly IA at once?
4. **Drop the identity-bundle layer** (Account direct, §3.3) — confirmed direction;
   schedule the resolver refactor + `_bundles` rename.
5. **Rename `db_memory_bundles` → `db_assemblies` / `db_identity_accounts` →
   `db_accounts`** now (clean) or defer the storage migration and rename the
   concept/UI first?

## 10. Recommendation

Adopt the **primitives + Assembly** model:

- **Account is the primitive**; drop the identity-bundle layer; "identity" is a
  derived view. Bind primitives **directly** to agents; an **Assembly** is an
  optional named **collection** for reuse/sharing.
- Name the collection **`Assembly`**; **remove the `_bundles` suffix** everywhere
  (`db_memory_bundles` → `db_assemblies`, `db_identity_accounts` → `db_accounts`,
  `db_identity_bundles` removed). Never "bundle" for a new concept.

Sequence: **(1)** break out MCP + Skills as first-class primitives (highest value,
lowest risk, the explicit ask) → **(2)** introduce **Assembly** as the reference-
based successor to Preset → **(3)** collapse identity-bundle → Account-direct +
resolver refactor → **(4)** backend `_bundles` rename + storage migration.
