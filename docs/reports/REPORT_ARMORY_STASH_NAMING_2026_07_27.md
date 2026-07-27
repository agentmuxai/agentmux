# Report: disambiguating the global Armory pane from the per-agent "agent armory" — naming + icon proposal

**Date:** 2026-07-27
**Author:** Agent3
**Verified against:** `main` @ `38978e6ba` (pulled 2026-07-27).
**Status:** Audit + naming proposal — not yet implemented, awaiting a name decision.
**Related:** `docs/specs/archive/SPEC_TRUST_CENTER_RENAME_2026_07_02.md` (original "Armory" naming brainstorm), `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` (execution), `docs/specs/SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md` (gave the per-agent button the same vault icon "for parity"), `docs/reports/REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md` (prior architecture/naming audit, 4 days before this one — did not address this specific two-surfaces-one-name problem).

## User's request (verbatim, for traceability)

> where are we in regard to the armory .. I am thinking we need a new name for the Agent's armory .. perhaps we can call it "Stash" ? so we can easily distinguish the armory pane and the per-agent accessible via the agent pane header .. what names, or icons do you propose? audit the current system

## 1. What "Armory" is today — one code concept, two surfaces sharing its name

**There is exactly one thing actually called "Armory" in the codebase and in user-facing copy: the global, fleet-wide pane** (`view: "armory"`, hamburger menu → Armory, `frontend/app/view/armory/`). It's a rail of five independently-owned managers over **shared, reusable** resources — Accounts, Memories, Skills, MCP Servers, Bundles — governed by an explicit architecture doc (`ARCHITECTURE_ARMORY_2026_07_20.md` §0): *"Armory holds shared, reusable resources... What deliberately does NOT live in Armory: per-agent-instance data."*

**The per-agent surface you're describing is `AgentSetupModal`** — opened by the single header icon on an agent pane (title/tooltip: **"Agent setup"**). It is a *separate* component tree (own Accounts/Memories/MCP Servers/Skills/Startup tabs), not the global pane reopened pre-filtered. Its user-facing button text has never said "Armory" — but three independent places in code and specs informally call it **"the agent armory"**:

- `AgentSetupModal.tsx:1-25`'s own doc comment.
- `SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md:26-31`, verbatim: *""The armory" in the original ask refers to `AgentSetupModal`... informally called "the agent armory" (a per-agent-scoped analogue of the global Armory pane)."*
- The header button's own code comment (`agent-model.ts:149-154`).

**So the ambiguity you're running into is real, but it currently lives in code comments, spec prose, and shared iconography — not in two on-screen labels both literally saying "Armory."** That's actually good news: there's no user-facing string to migrate, only an icon collision and an informal name that's leaking into how the team (and now you) talks about the feature. A name decision now, before "agent armory" calcifies into shipped UI copy, is well-timed.

## 2. The icon collision — the actual visible symptom today

Every Armory-adjacent surface deliberately reuses the same `fa-solid fa-vault` glyph, "for visual parity":

| Surface | Icon | File:line |
|---|---|---|
| Global Armory — hamburger menu | `vault` | `hamburger-menu.tsx:119` |
| Global Armory — widget/pane icon | `vault` | `agentmux-srv/src/config/widgets.json:211`, `armory-model.ts:22` |
| Global Armory — failure-banner "Open Armory → Accounts" action | `vault` | `failure-accessory.ts:104-107` |
| Per-agent header button ("Agent setup") | `vault` — **identical** | `agent-model.ts:149-155` |

This is the one place the two surfaces are genuinely indistinguishable at a glance: same glyph, same color, both reachable within a couple of clicks of each other (pane header vs. hamburger menu). A rename that doesn't also change this icon leaves the confusion fully intact — the label is secondary to the glyph for quick visual scanning. (For context: `vault` itself only exists because an earlier version of Armory used `shield-halved`, which collided with the **Warden** pane's icon — commit `9b88b7ca7` fixed that collision by switching to `vault`. Don't reintroduce a new collision picking the replacement.)

## 3. Prior naming history — directly relevant precedent

The 2026-07-02 brainstorm that produced "Armory" (`SPEC_TRUST_CENTER_RENAME_2026_07_02.md`) evaluated it as an **umbrella over multiple shared resource types**, and explicitly rejected two names worth knowing about before you pick a name for the *per-agent* surface:

- **"Vault"** — rejected for the umbrella: *"Credentials-only; undersells Brain + Presets + Identities."* Correctly avoided already (the per-agent modal never adopted it either).
- **"Loadout"** — runner-up, not rejected outright: *"The complete set an agent carries into a task... Slightly more 'the thing you assemble' than 'the place you manage it.'"* This objection was specifically about the *global* pane needing to read as a **place**, not a **thing one agent carries**. That objection doesn't apply to the per-agent surface — "this agent's loadout" is a *more* natural fit at the narrower, single-agent scope than it was at the umbrella scope. Worth surfacing as a real alternative, not just a footnote.

Also worth knowing: `REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md` §4 already reserves **"Connectors"** for a distinct, not-yet-built concept (preloaded MCP server integrations) and recommends against reusing it here — so it's off the table for this rename too, for the same reason.

## 4. Is "Stash" (or any short personal-kit name) actually a good semantic fit?

Mostly yes, with one honest caveat. Architecturally, `AgentSetupModal`'s tabs are not uniform in how "personal" they really are:

- **Accounts, Memories** — genuinely agent-scoped data (identity bindings, this agent's own memory files).
- **MCP Servers, Skills** — the modal is a *filtered view/binding* into the **same shared `db_mcp_servers`/`db_skills` tables** Armory itself manages, using hand-duplicated model code (`AgentMcpModel`/`AgentSkillModel` vs. Armory's `McpCatalogModel`/`SkillCatalogModel`) rather than genuinely separate, agent-owned copies (flagged as existing debt in the 07-23 report, not something this rename needs to fix).

"Stash" (and similarly, "Loadout") implies **this agent's own private collection**, which slightly oversells the MCP/Skills tabs' actual sharedness. In practice this is a minor, common naming simplification (a player's "inventory" in most games is also drawing from a shared item pool, not literally private) — not a reason to avoid the name, just worth a one-line clarifier in the modal's own copy ("bindings to shared Armory resources, scoped to this agent") if it's ever written, rather than implying true per-agent private storage.

## 5. Recommendation

**Keep "Armory" for the global pane** — it's shipped, well-established (three-week-old rename, zero regressions since), fits the Swarm/Warden/Drone register, and nothing in this request asks to touch it.

**For the per-agent surface (`AgentSetupModal`), ranked:**

1. **Stash** (your instinct) — top pick. Short, on-voice, reads as "this agent's personal kit" pulled *from* the Armory — a natural depot/personal-kit pairing (quartermaster's armory vs. a soldier's stash), distinct enough from "Armory" that the two can't be confused verbally, unlike a name that's a synonym or sub-word of "Armory" itself.
2. **Loadout** — strong runner-up, and arguably a *better* semantic fit than it would have been for the global pane (see §3) — "this agent's loadout" is exactly what the modal shows. Slightly more "assembled thing" than "place," which is fine at this scope (you open a modal, not a pane).
3. **Kit** — shortest option, clean, but generic enough that it undersells the identity/memory tabs (reads as "tools," similar undersell risk "Vault" had for the global pane).
4. **Locker** — "this agent's locker," familiar personal-storage metaphor, distinct from both Armory and Vault — reasonable fourth option if Stash/Loadout feel too game-y for your taste.

**Icon: `fa-solid fa-backpack`** for whichever name is chosen. Checked for collisions — `backpack`, `suitcase`, `toolbox`, `sack`, `locker` are all currently **unused** anywhere in the frontend or widget catalog (`vault`, `shield-halved`, `key`, `brain`, `plug`, `wand-magic-sparkles`, `layer-group`, `box` (Container runtime badge), `cog` (Settings), `id-card` (the button's *old*, pre-vault icon) are all already claimed elsewhere and worth continuing to avoid). A backpack reads as "personal gear you carry," is visually distinct from a vault's boxy/secure silhouette at a glance (the actual failure mode in §2), and keeps the same "equip an agent" metaphor register as Armory without duplicating its glyph family.

## 6. Suggested minimal-risk implementation scope (not yet done — awaiting your name pick)

Mirroring the original Trust Center → Armory rename's own "minimal-risk scope" precedent (`SPEC_TRUST_CENTER_RENAME_2026_07_02.md` §"Recommended minimal-risk scope"):

- User-facing: header button `title: "Agent setup"` → `title: "<Name>"` (`agent-model.ts:156`); `icon: "vault"` → `icon: "backpack"` (`agent-model.ts:155`).
- Non-user-facing, same PR for coherence: rename `AgentSetupModal` → `Agent<Name>Modal` (or keep the filename and just fix the doc comment — file rename is optional churn), update the "the agent armory" comments in `agent-model.ts`/`failure-accessory.ts`/the header-icon spec's own historical note (leave the spec's *history* section as-is; specs are a record, not live docs) to the new name.
- **Leave untouched:** `AgentMcpModel`/`AgentSkillModel`/RPC names/DB tables — same reasoning the original rename spec gave (internal key ≠ display name is already an accepted pattern in this codebase).
- Small bonus, same area, already flagged as debt independent of this rename: the `ArmorySection` id/label mismatch (`id:"brain"` renders "Memories," `id:"memories"` renders "Bundles," `armory-view.tsx:17-23`) — a natural one-line fix to bundle into the same PR if you want it, not required.

## File/line reference table

| Concern | File | Line(s) |
|---|---|---|
| Global Armory pane model (icon) | `frontend/app/view/armory/armory-model.ts` | 22 |
| Global Armory rail (labels+icons, id/label mismatch) | `frontend/app/view/armory/armory-view.tsx` | 17-23 |
| Armory hamburger entry | `frontend/app/window/hamburger-menu.tsx` | 117-120 |
| Armory widget def | `agentmux-srv/src/config/widgets.json` | 208-219 |
| Architecture/intent doc | `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md` | §0 |
| Per-agent header button (label + vault icon) | `frontend/app/view/agent/agent-model.ts` | 143-158 |
| Per-agent modal ("the agent armory") | `frontend/app/view/agent/components/AgentSetupModal.tsx` | 1-25 |
| Header icon rationale spec | `docs/specs/SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md` | 23-31 |
| Failure banner "Open Armory → Accounts" (vault reuse) | `frontend/app/view/agent/failure/failure-accessory.ts` | 100-108 |
| Naming brainstorm (Vault rejected, Loadout runner-up) | `docs/specs/archive/SPEC_TRUST_CENTER_RENAME_2026_07_02.md` | 36-53 |
| Icon collision precedent (vault replaced shield-halved to avoid Warden clash) | git commit `9b88b7ca7` | — |
| "Connectors" already reserved for a different concept | `docs/reports/REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md` | §4 |
| MCP/Skills hand-duplication (the §4 semantic caveat) | same report | §1, §2 |
