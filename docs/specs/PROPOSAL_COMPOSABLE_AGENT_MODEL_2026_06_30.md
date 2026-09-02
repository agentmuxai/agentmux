# Proposal: A Composable Agent Model for the Armory

**Date:** 2026-06-30
**Status:** Proposal — for discussion (naming + IA decisions needed)
**Author:** AgentX
**Driver:** "I'd like a cleaner model… break out skills and MCP into the Trust Center… should we call them bundles? preset bundles? would a preset be a one-stop shop?" *(verbatim — the Trust Center → Armory rename came later, PR #1917)*
**Related:** CLAUDE.md "Not widgets" (Identity / Presets); `db_memory_bundles`, `db_identity_bundles`; the per-agent identity provisioning work (`specs/archive/SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md`); the agent App API identity/preset/memory handlers.

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
from the thing that assembles them**, and give each a clear home in the
Armory.

## 2. Design principles

- **Primitives are first-class and reusable.** Define an MCP server, a skill, an
  identity, a brain, an instruction set **once**; reference it from many agents.
- **One bundle object is the "one-stop shop."** A single named thing answers
  "what is this agent's full setup" — by **referencing** primitives, not inlining
  them.
- **Reference, don't copy.** The bundle stores pointers; edit a primitive once
  and every agent using it updates.
- **The Armory is the trust + capability surface.** Accounts (credentials),
  MCP (external reach), and skills (injected instructions) are all *trust
  decisions* — they belong together where you grant and review capability.
- **Primitives bind directly; collections are optional.** An agent can bind
  primitives ad-hoc; a named **collection** is sugar for reuse, not a required
  wrapper.
- **Names should not collide, and shed legacy.** Don't reuse "bundle" for any new
  concept, and drop the misleading `_bundles` suffix on the storage names.

## 3. Proposed model — primitives + an optional collection

### 3.1 The primitives (Armory, each independently managed & shareable)

| Primitive | What it is | Today | Proposed home |
|---|---|---|---|
| **Account** | The provider login + credential the agent authenticates as (OAuth/key, per-account `CLAUDE_CONFIG_DIR`) | `db_identity_accounts`, wrapped in a redundant identity-**bundle** layer | Armory › **Accounts** (drop the bundle wrapper — §3.3) |
| **Memory** | Persistent learned knowledge (native memory store; the `memory.*` App API / `memory/` dir) | colloquially "the brain", per-account | Armory › **Memories** |
| **MCP server** | An external tool/connection surface (URL/stdio + which tools) | inlined in preset | Armory › **MCP Servers** (break out) |
| **Skill** | On-demand instruction/knowledge module (a folder) | inlined in preset | Armory › **Skills** (break out) |
| **Brief** | **The first message** injected when an agent pane opens — the startup payload (session context / identity / kickoff). That is *all* it is. It is **not** a standing instruction set; there is no always-on instruction blob. Behavioral/instructional content lives in **Skills** (on-demand). | the `startup` content type | Armory › **Briefs** |

> Each primitive carries its own **ownership/sharing** (agent-owned vs **global**,
> mirroring the App API's existing global-preset guard) and its own audit trail.
> Accounts and MCP servers especially are security-sensitive — first-class status
> gives them proper review, not burial inside a preset blob.

### 3.2 Direct binding is the base; a Bundle is just a named collection

The base mechanism is **direct binding**: an agent binds the primitives it needs
(any number of MCP servers + skills, one Brief, ≤1 Account per provider, a Memory).
A **Bundle** is **not a required wrapper** — it's a *named, reusable collection*
of those same bindings, for "apply this whole set in one step" and "share it across
agents." It's convenience + reuse, not a new layer the agent must have.

```
Bundle "AgentX"            (a saved collection — optional)
  ├─ accounts: [Accounts/AgentX-claude, Accounts/AgentX-github, Accounts/AgentX-aws]
  ├─ memory:   → Memories/AgentX
  ├─ mcp:      → [MCP/github, MCP/agentmux, MCP/aws]
  ├─ skills:   → [Skills/git-workflow, Skills/reagent, …]
  └─ brief:    → Briefs/host-agent
```

- It holds **references**, not copies (principle: reference, don't copy).
- **An Agent's effective config = its direct bindings ∪ the Bundles it
  includes.** Precedence: a direct binding overrides the same item from a Bundle
  (so a Bundle is a baseline you can locally tweak).
- Provider/model and workspace stay on the **agent** (not portable across
  machines/providers the way the bound primitives are) — preserving the current
  "provider/model belong to the agent" rule.

**Yes — a Bundle is the one-stop shop when you want one.** But because primitives
bind directly, you can also run an agent with no Bundle at all (ad-hoc bindings),
or mix a Bundle with a couple of direct overrides. That flexibility is the
difference from today's mandatory, inline preset.

### 3.3 Do we still need "Identity"? No — Account is the primitive

Identity today is **three layers**: `bundle → bindings → accounts`, where a
**bundle is itself a collection** — exactly one account *per provider*
(claude→A, github→B, aws→C). That is *literally a bundle of accounts*. Once the
Bundle concept exists, the identity-bundle layer is **redundant**:

- The real primitive is the **Account** (provider + credential). Drop the
  identity-**bundle** object entirely.
- Agents bind **accounts directly**. The only invariant the bundle used to enforce
  — **≤1 account per provider** — moves to **resolution time** (the resolver picks
  the single account per provider from the agent's direct bindings + bundles;
  a conflict is a validation error, surfaced like any other).
- A named, reusable set of accounts is just an **Bundle** (which can hold accounts
  alongside MCP/skills/brief).
- "**Identity**" survives only as a *user-facing descriptor* — "what is this agent
  running as" — a derived view over the agent's bound accounts, **not a stored
  object**.

This also **simplifies the in-flight provisioning spec**
(`specs/archive/SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md`): an Account already carries
its own `OAuthConfigDir`, so there is no bundle to provision — log in → get an
Account → bind it. The resolver change (instance → account directly, instead of
instance → bundle → binding → account) is the one real refactor; flag it there.

### 3.4 What loads when (verified against the spawn path)

**The Brief is *just the first message* the pane opens with** — nothing more. There
is deliberately **no always-on standing-instruction blob**. Traced through the
current spawn path:

| Primitive | When | Mechanism (today) |
|---|---|---|
| **Brief** | **the first message, at pane open** | the `startup` content is injected as the **first user message** (`buildStartupPayload()` → `AgentInputCommand` → CLI stdin, `subprocess.rs:494`) — session context / identity / kickoff |
| **Memory** | recalled at startup | native memory store, per-account `CLAUDE_CONFIG_DIR` |
| **Skill** | **on-demand** | indexed + `.claude/commands/<trigger>.md`; loaded only when invoked. **Instructional/behavioral content lives here** — not in an always-on prompt |
| **MCP server** | connection at startup, tools on-demand | `.mcp.json` (`build_mcp_config`, auto-injects the `agentmux-mcp` entry) |
| **Account** | at pane open | "Assigned Accounts" in the startup payload + identity resolution (`resolver.rs`) |

> **Deliberate stance: no always-on instruction set.** Today the `soul`+`agentmd`
> content is written to an always-loaded **`CLAUDE.md`** (`agent_config.rs:55-96`).
> We are **retiring that** — heavy standing instructions ("that terminality") are
> not wanted. **Brief = the first message; everything instructional moves to
> Skills** (on-demand). So `soul`/`agentmd` migrate into Skills, not into Brief.
> (Citations: `frontend/.../startup/buildStartupPayload.ts`, `subprocess.rs:494`,
> `server/agent_config.rs:55-96`, `server/app_api.rs:2299-2405`.)

**Two content types the model must place** (surfaced by the trace — §9):
- **`hooks` + `settings`/permissions** → `.claude/settings.json`. Behavior/safety
  config that loads at startup. *Recommend a distinct **Policy** primitive* —
  permissions are a trust decision and deserve their own review surface.
- **`soul` / `agentmd` (today's CLAUDE.md instructions)** → **migrate into Skills**
  (on-demand), per the stance above — not Brief, not a standing prompt.
- **the static `memory` content blob** (in CLAUDE.md today) is **not** the native
  Memory store → migrate into native **Memory**; don't carry two "memory" concepts.

## 4. Naming (the crux of the question)

Two things need names: **the bundle** and **the broken-out primitives**.

### 4.1 The collection — candidates

| Name | For | Against |
|---|---|---|
| **Bundle** *(recommended — product owner's call)* | The plain English word for "a set of things shipped/applied together" — exactly what this is; familiar; and *freed of collision* by this proposal (see §4.3) | had a collision under the old names — removed below |
| **Profile** | groups capabilities; familiar (browser profiles, **AWS_PROFILE**) | connotes a *persona* more than a *collection*; less apt now that it's an optional collection, not a wrapper |
| **Assembly** | accurately means "a collection assembled from parts" | a touch jargony; loses to "Bundle" on plainness once the collision is gone |
| **Preset** (redefine) | incumbent | currently the *narrow* inline thing; stretching it muddies the rename |

**Recommendation: name the collection a `Bundle`.** It is the most natural word for
the concept, and — critically — this proposal *eliminates every other meaning of
"bundle"* (§4.3), so it lands on exactly one referent. Retire "preset" (keep as a
read alias for one release).

### 4.2 The primitives — naming

Keep them concrete and singular: **Account, Memory, MCP Server, Skill, Brief**.
(**Memory** matches the native concept + the `memory.*` App API — no UI/backend
split; "the brain" stays a colloquial nickname, not the canonical term. The reason
"Brain" was ever needed — disambiguating from `db_memory_bundles` — is removed by
this proposal's `db_memory_bundles` → `db_bundles` rename. **`Brief` is the name —
not "Instructions"**: it is deliberately *not* a standing instruction set, it's the
opening message; "Instructions" would mislabel it and re-imply the always-on prompt
we're removing.) Note: **there is no "Identity" primitive** — Account is the
primitive; "identity" is a derived view (§3.3).

### 4.3 Why "Bundle" works now (the collision is removed, not worked around)

The earlier objection to "bundle" was **collision** — it already meant two things
(`db_identity_bundles`, `db_memory_bundles`). But this proposal **deletes both of
those meanings**:

- `db_identity_bundles` → **gone** (the identity-bundle layer collapses to Accounts,
  §3.3).
- `db_memory_bundles` → renamed (it was never memory and never a real "bundle").

So instead of *adding* a third meaning, we **consolidate to one**: after the
cleanup, "bundle" refers to exactly **the collection** — its plain English sense.
That inverts the old objection: removing `_bundles` from the legacy names is
precisely what *frees* the word for its natural use. `db_memory_bundles` →
**`db_bundles`** (the collection store); "bundle" means the collection, full stop.

## 5. Armory information architecture

```
Armory
├─ Bundles      ← named collections you apply to agents (optional, reusable)
├─ Accounts        ← provider logins / credentials (per-account CLAUDE_CONFIG_DIR)
├─ Memories        ← memory stores
├─ MCP Servers     ← external tool/connection surfaces  (NEW: broken out)
├─ Skills          ← instruction/knowledge modules       (NEW: broken out)
└─ Briefs          ← the opening message a pane loads with (no standing prompt)
```

No **Identities** tab — Account is the primitive; "identity" is a derived view
(§3.3). A **Bundle** view shows reference slots and lets you pick from the
primitive lists; "create new" from a slot deep-links to the primitive editor. An
agent binds primitives directly and/or includes Bundles, plus picks model +
workspace.

## 6. Trust & sharing model

- Each primitive is **agent-owned** or **global** (shared). The App API already
  guards global presets (`preset.upsert` rejects modifying `is_global`); generalize
  that guard to **every** primitive: an agent may write its own, never another's or
  a global one without authorization. (Ties directly to the S1/S4 work in the agent
  App API.)
- **Accounts** and **MCP servers** carry the heaviest trust weight (credentials +
  external reach). First-class status means they get explicit grant/review in the
  Armory instead of riding hidden inside a preset.
- An Bundle referencing a global primitive can't silently fork it; it points at
  the shared definition.

## 7. Backend / migration sketch

- **Reference model:** Bundles store primitive **IDs**, not inline JSON. Migrate
  today's inline presets by extracting their MCP/skills into standalone
  `mcp_servers` / `skills` rows and rewriting the preset as references → an
  `Bundle`.
- **Consolidate "bundle" to one meaning** — today it's spread across two misleading
  tables; collapse to a single correct one:
  - `db_memory_bundles` → `db_bundles` — today's name says "memory" but it holds
    presets; the renamed table *is* the collection store, so "bundle" finally fits.
  - `db_identity_bundles` → **removed**: the identity-bundle layer collapses (§3.3);
    agents/bundles reference accounts directly.
  - `db_identity_accounts` → `db_accounts`.
  - Phase it: rename the **concept/UI now** (cheap), do the **storage migration**
    behind the App API later so agents only ever see the clean surface.
- **Resolver change:** spawn resolution moves from `instance → identity_bundle →
  binding → account` to `instance/bundle → account` directly (one account per
  provider, enforced at resolve). Reconcile with
  `specs/archive/SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md`.
- **Compatibility:** keep "preset" / `db_memory_bundles` as a read alias for one
  release; new writes go to Bundles.
- **App API:** add `mcp.*`, `skill.*`, and `account.*` agent commands paralleling
  the existing `memory.*`, with the same S1 + ownership/global guards. `preset.*`
  becomes `bundle.*`.

## 8. What this unlocks (incl. the claw import)

The `a5af/claw` "AgentX/AgentY" scheme maps **cleanly** onto this model — it stops
being an awkward "dump it in memory" and becomes:
- **Brief:** the claw startup message only (the kickoff). The claw `CLAUDE.md` /
  startup-prompt *instructions* become **Skills** (on-demand), not the Brief
- **Skills:** the claw `templates/skills/*` (each a first-class Skill)
- **MCP Servers:** the claw `.mcp.json` entries (each a first-class MCP server)
- **Account:** AgentX's Claude login (the per-agent provisioning work)
- **Memory:** AgentX's accumulated memory
- → collected into a **Bundle "AgentX"**, applied to the AgentX agent (and shared
  with AgentY where identical).

That's the "clean model" the import wanted: reusable, shareable, reviewable
building blocks — not config smeared across three stores.

## 9. Open decisions (need product-owner input)

1. **Collection name:** `Bundle` (recommended; product owner's call) vs. `Profile`
   / `Assembly`. Confirm.
2. ~~"Brief" vs "Instructions"~~ — **settled: `Brief`, defined as the opening
   message only**; no "Instructions" concept; standing instructions → Skills.
3. **Scope of v1:** break out **MCP + Skills** first (the explicit ask), or land the
   whole primitives + Bundle IA at once?
4. **Drop the identity-bundle layer** (Account direct, §3.3) — confirmed direction;
   schedule the resolver refactor + `_bundles` rename.
5. **Rename `db_memory_bundles` → `db_bundles` / `db_identity_accounts` →
   `db_accounts`** now (clean) or defer the storage migration and rename the
   concept/UI first?
6. **Home for hooks + permissions** (`.claude/settings.json`): a distinct **Policy**
   primitive (recommended — permissions are a trust decision) vs. fold into Brief?
7. **Static `memory` content blob** (in CLAUDE.md today): merge into Brief
   (recommended) vs. migrate into the native Memory store? Either way, retire the
   second "memory" concept.

## 10. Recommendation

Adopt the **primitives + Bundle** model:

- **Account is the primitive**; drop the identity-bundle layer; "identity" is a
  derived view. Bind primitives **directly** to agents; a **Bundle** is an optional
  named **collection** for reuse/sharing.
- Name the collection **`Bundle`** — and consolidate "bundle" to that one meaning:
  `db_memory_bundles` → `db_bundles` (the collection store), `db_identity_accounts`
  → `db_accounts`, `db_identity_bundles` removed. After cleanup "bundle" refers to
  exactly one thing.

Sequence: **(1)** break out MCP + Skills as first-class primitives (highest value,
lowest risk, the explicit ask) → **(2)** introduce **Bundle** as the reference-
based successor to Preset → **(3)** collapse identity-bundle → Account-direct +
resolver refactor → **(4)** backend `_bundles` rename + storage migration.
