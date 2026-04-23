# Spec: Agent Pane — Definition Cards + Launch Modal

**Date:** 2026-04-23
**Status:** Draft
**Owner:** AgentA
**Related:**
- [discussion #493 — research on 7 AI coding CLIs](https://github.com/agentmuxai/agentmux/discussions/493)
- [SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md](./SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md) — current 3-button action bar
- [default-agent-roster.md](./default-agent-roster.md) — host/container seed rosters
- [container-agent-runtime.md](./container-agent-runtime.md) — container execution layer
- [portable-agent-working-dirs.md](./portable-agent-working-dirs.md) — working-directory resolution
- [SPEC_FORGE_IDENTITY_AGENT_INSTANCES_2026_04_20.md](../../specs/SPEC_FORGE_IDENTITY_AGENT_INSTANCES_2026_04_20.md) — definition vs instance model

---

## 1. Mental model (the one-liner that drives everything)

> **Each card in the agent pane is a *definition* (one per CLI). Clicking a card opens a modal to spawn a named *instance* of that definition.**

Today the UI blurs this line: clicking a card launches the underlying CLI into the current pane immediately, and the card's title reads like a user-named agent (e.g. "AgentX"). That hides the fact that every launch is a fresh instance with its own working directory, session id, and process lifetime.

This spec makes the split explicit:

- **Definition** — a seeded, CLI-specific template. Seven of them, one per CLI surveyed in discussion #493.
- **Instance** — what a definition *becomes* when you click it + name it. Has a working directory, a session id, and shows up in swarm.

## 2. Goals

- **G1.** Agent-pane cards are **definition tiles** — each is keyed on a CLI (claude, codex, gemini, kimi, pi, openclaw, copilot). Every card leads with a short description of what the CLI *does*; the CLI's brand name sits as a caption below.
- **G2.** Clicking a definition card opens a **Launch modal** that asks for: instance name + host/container runtime. Submit launches the instance in the current pane.
- **G3.** The instance's working directory is `<slug(name)>-<YYYYMMDD-HHMMSS>` under `AGENTMUX_DATA_HOME`. Two instances with the same name never collide.
- **G4.** Launched instances are re-accessible from the **swarm** pane.
- **G5.** Drop the "Add Agent / Add Definition" button from the pinned bottom bar. Definitions are seeded; users don't author new ones from the agent pane.
- **G6.** Use popovers liberally — every non-obvious label gets a `ⓘ` with a quick explanation (CLI details come from discussion #493).

## 3. Non-goals

- Authoring new definitions from scratch (that lives in the Forge pane and is out of scope here).
- Implementing the container runtime — already specced in [container-agent-runtime.md](./container-agent-runtime.md). This spec only drives the `agent_type` / `environment` fields.
- Changing the identity / account flow. Identity attachment stays where it is today.
- Renaming the underlying `ForgeAgent` type. "Definition" is a UI term; the row schema is untouched.

---

## 4. The agent pane today vs after

```
BEFORE (today)                         AFTER (this spec)
──────────────                         ─────────────────

┌─────────────────────────────┐        ┌─────────────────────────────┐
│ AgentX            ✎ ⚙ 👤 🗑│        │ Anthropic's coding agent   │
│ ✖ agentx                    │        │ Claude Code        ⓘ ✎ ⚙ 🗑 │
│ Claude Code on host         │        │                             │
│                             │        │ OpenAI's coding agent       │
│ AgentY            ✎ ⚙ 👤 🗑│        │ Codex CLI          ⓘ ✎ ⚙ 🗑 │
│ ✦ agenty                    │        │                             │
│ Codex CLI on host           │        │ Google's coding agent       │
│ …                           │        │ Gemini CLI         ⓘ ✎ ⚙ 🗑 │
├─────────────────────────────┤        │ …                           │
│ [+ Add Agent] [↓] [↑]       │        ├─────────────────────────────┤
└─────────────────────────────┘        │ [↓ Import] [↑ Export]       │
                                       └─────────────────────────────┘
  click → launch immediately             click → Launch modal
  (name is "AgentX", working dir         (user names the instance,
   derived from slug without stamp)       picks host/container, then launch)
```

Key differences:

- **Card title is a description**, not the bespoke agent name. The CLI brand is the small caption.
- **No per-card name/slug pair** — there's nothing to display, definitions don't have user-chosen names.
- **Bottom bar loses the Add button** — only Import / Export remain. (Rationale: definitions are seeded once; new ones are added via Forge or by editing seed data, not from this surface.)
- **👤 identity button removed from the card** — identity is a per-*instance* concern and lives in the modal / settings panel.
- **Click semantics change** — now opens the Launch modal instead of launching.

### 4.1 Definition card anatomy

Each card is driven by a single entry from the CLI catalog (§5). Visual layout:

```
┌─────────────────────────────────────────────────┐
│ [icon]  Anthropic's coding agent      ⓘ ✎ ⚙ 🗑 │
│         Claude Code  ·  reads CLAUDE.md         │
└─────────────────────────────────────────────────┘
```

- **Icon (left):** unicode glyph from the catalog.
- **Title line:** capability-first description (e.g. *"Anthropic's coding agent"*).
- **Caption line:** CLI brand name · primary context file badge (e.g. *"Claude Code · reads CLAUDE.md"*).
- **Action buttons:** `ⓘ` popover (deep CLI detail from discussion #493), `✎` rename description (rarely used), `⚙` forge/identity settings, `🗑` delete definition.
- **Click the body:** opens the Launch modal.

The `ⓘ` popover is the highest-value addition on the card: one hover gives the user the startup-flow summary, context-file behaviour, MCP support, memory model, and version — all pulled from discussion #493's tables.

---

## 5. CLI catalog

A single source of truth, seeded from discussion #493 and checked into source as `frontend/app/view/agent/defaults/cli-catalog.ts`. The Forge seed writes one `ForgeAgent` row per catalog entry on first run.

```typescript
export interface CliCatalogEntry {
    provider: string;               // ForgeAgent.provider — "claude" | "codex" | …
    displayName: string;            // "Claude Code"
    icon: string;                   // unicode glyph
    blurb: string;                  // "Anthropic's coding agent"
    primaryContextFile: string;     // "CLAUDE.md" | "AGENTS.md" | "GEMINI.md"
    mcpSupport: "stdio+http" | "stdio+http+oauth" | "none";
    popoverMarkdown: string;        // longer description for ⓘ popover
    defaultFlags: string[];         // seeds provider_flags
    hostSupported: boolean;
    containerSupported: boolean;
    containerImage?: string;        // default image when container mode selected
}
```

Initial entries — one per CLI from discussion #493:

| provider | displayName | blurb | primaryContextFile |
|---|---|---|---|
| `claude` | Claude Code | Anthropic's coding agent | `CLAUDE.md` |
| `codex` | Codex CLI | OpenAI's coding agent | `AGENTS.md` |
| `gemini` | Gemini CLI | Google's coding agent | `GEMINI.md` |
| `kimi` | Kimi Code | Moonshot's 262k-context agent | `AGENTS.md` |
| `pi` | Pi | Plandex's multi-provider agent | `AGENTS.md` / `CLAUDE.md` |
| `openclaw` | OpenClaw | ACP orchestration platform | `AGENTS.md` + family |
| `copilot` | GitHub Copilot CLI | Microsoft's coding agent | `AGENTS.md` + `.github/*` |

Catalog updates are a manual PR — when discussion #493 evolves, the catalog evolves in lockstep.

---

## 6. Launch modal

Opens on click of a definition card. Three short sections:

```
┌─────────────────────────────────────────────────────────────┐
│ Launch Claude Code                                      [×]│
│                                                             │
│ Anthropic's coding agent. Reads CLAUDE.md at repo root,    │
│ honours .claude/rules/**, has stdio + http MCP.      ⓘ     │
│                                                             │
│ Name                                                   ⓘ    │
│ [ _________________________________ ]                      │
│                                                             │
│ Runtime                                                ⓘ    │
│ ◎ Host (your machine)    ○ Container (isolated)            │
│ [Image: agentmux/claude:latest ▼]   (if Container)         │
│                                                             │
│ Working dir: data/agents/myname-20260423-152001/            │
│                                                             │
│                                     [Cancel]  [Launch]     │
└─────────────────────────────────────────────────────────────┘
```

### 6.1 Fields

| Field | Required | Validation | Popover content |
|---|---|---|---|
| Name | yes | 1–64 chars, `[A-Za-z0-9 _-]+`, unique within this definition's instances | "The human-readable label for this run. The working directory uses a slug of this name + a timestamp." |
| Runtime | yes | `host` default; `container` only enabled if runtime detected | "Host = runs on your OS. Container = runs in Docker/Podman — slower startup but sandboxed." |
| Image | only if `Container` | free text + dropdown with curated tags | "Container image the CLI runs inside. Leave default unless you know what you're doing." |

### 6.2 Behaviour

- **Submit** calls `RpcApi.LaunchAgentInstance(...)` (thin wrapper around the existing launch flow, passing the modal values). Backend:
    1. Resolves `working_directory` = `<data_home>/agents/<slug>-<YYYYMMDD-HHMMSS>/`.
    2. Creates the directory.
    3. Launches the CLI with the provided `agent_type` + `environment`.
    4. Returns the new instance id.
- **Keyboard:** Enter submits if the form is valid; Escape cancels.
- **Preview line** for the working directory uses frontend-local time as a courtesy; the backend computes the final stamp on submit to avoid drift if the modal stays open.
- **Clicking a definition card with the modal already open** cancels the previous choice and re-opens with the new definition.

### 6.3 Popovers

Every non-obvious surface gets a `ⓘ`:

- **Card body (above the modal)** — startup flow summary pulled from discussion #493.
- **Name label** — naming rules + working-dir impact.
- **Runtime label** — host vs container trade-offs.
- **Image dropdown** — what an image is, when you'd pick a non-default one.
- **Working-dir preview** — why we stamp with the time (answer: isolation + uniqueness).

Popover rules: ≤ 280 chars body; "Learn more →" links out for longer content; close on outside click or Escape; never modal-blocking.

---

## 7. Working-directory resolution

Format:

```
<data_home>/agents/<slug>-<YYYYMMDD-HHMMSS>/
```

- `<data_home>` = `AGENTMUX_DATA_HOME` per [portable-agent-working-dirs.md](./portable-agent-working-dirs.md). Portable → `<portable>/data/`; installed → `$HOME/.agentmux/`.
- `<slug>` = lowercase, spaces → `-`, strip everything not `[a-z0-9-]`. Same logic as `AgentPicker.handleRename`.
- `<YYYYMMDD-HHMMSS>` = local time at submit. Local, not UTC, so the folder timestamps match the user's own clock when they browse on disk.

Two instances with identical names get distinct directories because the timestamp differs by at least one second. If the user manages to hit sub-second collision, the backend appends `-1`, `-2`, … until the path is free — simple retry loop, no UUIDs.

Renaming a definition later does **not** rename existing instance dirs — they're tied to the instance id, not the definition.

---

## 8. Bottom bar changes

`AgentActionBar.tsx` shrinks from 3 buttons to 2:

```diff
- [+ Add Agent]  [↓ Import]  [↑ Export]
+ [↓ Import]                [↑ Export]
```

- `handleAddAgent` and its signal are removed.
- Import / Export unchanged.
- Button layout switches to `justify-content: space-between` so the two remaining buttons spread instead of stacking left.

Rationale: adding a new *definition* is a rare, power-user task that belongs in the Forge pane; cluttering the common agent-pane footer with a creation entry point isn't worth the discoverability. If users ask for it, a small "+" in the card grid is a cheaper follow-up than a dedicated button.

---

## 9. Card click — the key behavior change

Today (`AgentPicker.handleSelect`):

```typescript
const handleSelect = async (agent: ForgeAgent) => {
    await props.model.launchForgeAgent(agent);
    // …
};
```

After:

```typescript
const handleSelect = (agent: ForgeAgent) => {
    setLaunchModalAgent(agent);  // open modal, don't launch yet
};

const handleLaunchSubmit = async (agent, name, runtime) => {
    await props.model.launchForgeAgent(agent, {
        instanceName: name,
        agentType: runtime.type,     // "host" | "container"
        environment: runtime.env,    // "local" | "docker"
        containerImage: runtime.image,
    });
    setLaunchModalAgent(null);
};
```

`launchForgeAgent` already accepts overrides for `working_directory` / `agent_type` / `environment` — see `agent-model.ts`. The modal just supplies them from user input instead of inheriting from the definition's seed values.

---

## 10. Swarm re-access

Every launched instance appears in the **swarm pane's instances list**:

- icon + instance name + runtime badge (HOST / CONTAINER) + live/idle/crashed status
- click → focuses the pane running that instance (if still open), or opens a new pane and attaches
- right-click → stop, restart, open working dir, delete

This is the single place users go to "find that agent I started earlier." Implementation extends the existing swarm pane — piggybacks on the process-tracker series (PR #497–501) so every instance is already tracked.

The definition list (seven entries) lives in the agent pane only. Swarm is exclusively about running / historical instances.

---

## 11. Implementation plan

Small, independently landable PRs, gated.

### PR A — Bottom-bar: drop "+ Add Agent"
- Remove button + `handleAddAgent` from `AgentActionBar.tsx`.
- Re-flow the two remaining buttons.
- ~20 LOC. **Gate:** none.

### PR B — Rework `AgentCard` layout
- Title line = `agent.description` (seeded from catalog blurb); caption line = `agent.name` + primary-context badge.
- Remove the 👤 Identity button from the card (moved to settings panel only).
- Add a `ⓘ` popover slot; content comes from the catalog entry.
- **Gate:** none (visual-only).

### PR C — CLI catalog module
- Add `frontend/app/view/agent/defaults/cli-catalog.ts` with the 7 entries.
- Hook `useCliCatalog()` returns the catalog; future: merge with backend-delivered catalog.
- Backend Forge seed rewritten to use catalog entries → ensures card metadata and seed data agree.
- **Gate:** PR B (so cards can display catalog values).

### PR D — Launch modal (name + runtime)
- New component `AgentLaunchModal.tsx` + SCSS.
- Wire `handleSelect` to open the modal; submit path calls the updated `launchForgeAgent` with overrides.
- `launchForgeAgent` signature extended to accept `LaunchOverrides` (backward-compatible: all fields optional).
- Unit tests: validation, submit path, Escape/Enter behavior.
- **Gate:** PR C.

### PR E — Working-dir timestamp in backend
- Backend `forge_handlers::launch_agent` resolves `working_directory` to `<slug>-<YYYYMMDD-HHMMSS>` when the frontend passes an override without an explicit dir.
- Collision handling: `-1`, `-2` suffix loop.
- Test: two launches one second apart → two distinct dirs.
- **Gate:** PR D.

### PR F — Popovers everywhere
- Reuse the existing popover primitive (check `frontend/app/element/` for the pane-header one, else add a tiny wrapper).
- Add `ⓘ` on: card body, modal fields, working-dir preview.
- Content sourced from catalog `popoverMarkdown` + short helper strings.
- **Gate:** PRs B + D.

### PR G — Swarm "Instances" surface
- Extend the swarm pane with an instances list driven by the process tracker + instance table (per SPEC_FORGE_IDENTITY_AGENT_INSTANCES).
- Click → focus or open pane for that instance.
- **Gate:** PRs D + E.

### PR H — Polish pass
- Copy review on popovers, tooltips, and modal labels.
- Keyboard shortcut audit (Enter submits, Escape cancels, Tab order).
- Screenshot test for the new card layout.
- **Gate:** all above.

---

## 12. Open questions

1. **What fills the Name field by default?** Leaving it blank forces the user to type; prefilling with the CLI's brand name (e.g. "Claude Code") is friendlier but encourages default-accepting. Leaning **blank with the brand name as placeholder text** — cheap compromise.
2. **Container image catalog.** Ship a small curated list (`agentmux/claude:latest`, `agentmux/codex:latest`, …) or freeform-only? Recommendation: curated list + `Custom…` option; the "which image?" question is the exact kind of thing popovers should answer.
3. **Re-launching a closed instance from swarm.** Does the swarm pane offer "duplicate" (create a new instance with a new stamp) or "resume" (attach to the same working dir)? Probably both — but that's a swarm-pane spec concern, not this one.
4. **Deleting a definition with live instances.** Today, definitions and instances share a row; once they diverge, we need a guard: "Can't delete — 2 instances still running." Swarm PR handles this.

---

## 13. Rollout & metrics

- No feature flag needed — all changes are UI-side + additive backend RPC fields.
- Success signal #1: time from "open agent pane" to "agent responding" stays flat (modal adds one click but removes ambiguity about which CLI was launched).
- Success signal #2: number of duplicate working-dir errors in logs → 0 after PR E.
- Follow-up: measure swarm-pane opens per day post-PR G.

---

## 14. Cross-references

- Discussion #493 — source for the CLI catalog. Catalog updates are lockstep.
- `SPEC_FORGE_IDENTITY_AGENT_INSTANCES_2026_04_20.md` — formalises the definition-vs-instance split this spec cements at the UI layer.
- `container-agent-runtime.md` — container path the runtime picker eventually exercises.
- `portable-agent-working-dirs.md` — `AGENTMUX_DATA_HOME` resolution used in §7.
- `SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md` — action bar spec this one supersedes (button removed).
