# Spec: Rename "Trust Center"

> **Archived 2026-07-12.** Superseded by `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md` — this was the naming brainstorm; the linked doc is the resolved implementation spec. Consolidated tracking: issue #2024.

**Date:** 2026-07-02
**Status:** Proposal — brainstorm + rename plan for review
**Author:** Agent1

---

## Why rename

"Trust Center" is the most corporate/formal name in AgentMux. The rest of the product speaks in short, role-forward, metaphor-friendly names — **Swarm** (coordination), **Warden** (oversight), **Drone** (execution), plus **muxbus**, **jekt**, **brain**. "Trust Center" reads like an enterprise SaaS console and clashes with that voice. It's also vague: nothing about "Trust Center" tells a new user it's where credentials, identities, shared knowledge, and per-agent config live.

## What it actually is (scope the name must cover)

The Trust Center is the umbrella surface for the reusable building blocks that define **who an agent is and what it may touch**. Four tabs:

| Tab | What it manages |
|-----|-----------------|
| **Accounts** | Credential library — API keys, OAuth tokens, PATs, IAM roles. Stored in OS keychain (never plaintext); validated live on demand; injected as env vars (`GH_TOKEN`, `ANTHROPIC_API_KEY`, …) at agent spawn. |
| **Identities** | Named bundles of accounts, reused across agents; bindings in `db_identity_bindings`. |
| **Brain** | Workspace-wide markdown (coding standards, security rules) concatenated into every agent's `CLAUDE.md` at spawn. |
| **Presets** | Per-agent config bundles (instructions, context files, MCP servers, skills). |

So the umbrella name should signal: **"here is where you equip and define your agents — their credentials, identity, knowledge, and config."**

## Naming criteria (from AgentMux's spirit)

1. Short — 1 word ideally, 2 max.
2. Role- or metaphor-forward, not abstract corporate ("Hub", "Center", "Platform", "Manager", "Suite" are out).
3. Fits alongside Swarm / Warden / Drone without feeling out of place.
4. Works as an **umbrella** over all four tabs — not just credentials (rules out "Vault", "Keyring") and not just identity (rules out "Roster", "Identity").
5. Evokes *equipping / provisioning an agent*, since that's the throughline across all four tabs.

## Candidates

### Recommended: **Armory**
The place where an agent's gear is kept and issued. Credentials are the tools/keys, identities are the loadouts, brain + presets are the standard-issue kit. Reads naturally as an umbrella ("open the Armory"), sits perfectly beside **Warden** and **Drone** (shared martial/ops register), and is a *place* — a clean 1:1 swap for "Trust Center" as a destination. Short, concrete, on-voice.

### Strong alternative: **Loadout**
The complete set an agent carries into a task — credentials + identity + knowledge + config *are* the loadout. Action-forward and gaming-adjacent like Swarm/Drone. Slightly more "the thing you assemble" than "the place you manage it"; works well if we ever reframe the surface around "assemble this agent's loadout."

### Other contenders
| Name | Read | Why it's lower |
|------|------|----------------|
| **Quartermaster** | The role that issues equipment/credentials. Very on-voice (Warden-like role name). | Long (12 chars); better as a persona than a menu label. |
| **Rig** | "Rig up an agent." Short, punchy. | Reads more hardware/terminal than identity+knowledge. |
| **Provisions** / **Provision** | "Provision your agents." Covers all four tabs literally. | A bit utilitarian; verb/noun ambiguity. |
| **Vault** | Clean, secure. | Credentials-only; undersells Brain + Presets + Identities. Rule-4 fail. |
| **Cabinet** | Filing/storage, accessible. | Formal-ish, closer to the "Center" vibe we're leaving. |

**Recommendation: `Armory`**, with `Loadout` as the fallback if the team prefers the "the thing" framing over "the place."

## Rename impact (touchpoints)

### Must change together (user-facing)
- Hamburger menu entry — `frontend/app/menu/hamburger-menu.tsx` (~line 65): `"Trust Center"`
- Widget label — `agentmux-srv/src/config/widgets.json` (~line 212): `"Trust Center"`
- Widget description (~line 213): `"Manage accounts, identities, brain, and presets"` — keep or lightly refresh
- Keep the four tab labels (**Accounts / Identities / Brain / Presets**) as-is — they're clear and independent of the umbrella name.
- Keep the icon consistent across menu + widget.

### Can change independently, non-user-facing (do in the same PR for coherence, or defer)
- Component names: `TrustViewModel`, `TrustView`, `TrustSection` → e.g. `ArmoryViewModel`, `ArmoryView`, `ArmorySection`
- View type key `view: "trust"` and widget key `defwidget@trust` — **these are persisted in pane metadata**; renaming requires a migration/back-compat shim (the `forge` → `agent` redirect in `block.tsx` is the precedent). **Recommendation: keep `view: "trust"` as the internal persisted key** to avoid a migration, and only rename the user-facing strings + optionally the TS component names. Internal key ≠ display name is already an accepted pattern (backend keeps `db_memory_bundles` while UI says "Presets").
- RPC command names (`account.key.verify`, `account.oauth.*`) and DB tables (`db_identity_accounts`, `db_identity_bundles`) — **leave unchanged**; they're internal and renaming them is churn with contract-test + migration cost for zero user benefit.

### Recommended minimal-risk scope
1. Rename the **user-facing strings only** (hamburger label, widget label, optional description). ~3 strings.
2. Optionally rename the **TS component/type names** (`Trust*` → `Armory*`) for code clarity — pure rename, no persisted-state impact.
3. **Do NOT** touch `view: "trust"`, `defwidget@trust` persisted keys, RPC names, or DB tables — internal, keeping them avoids migrations (precedent: Presets/`db_memory_bundles`, Forge shim).

This delivers the whole visible rename with essentially zero migration risk.

## Open question for the human

Pick the name: **Armory** (recommended, "the place") vs **Loadout** ("the thing") vs another. Everything downstream (strings, optional component renames) follows mechanically once chosen.
