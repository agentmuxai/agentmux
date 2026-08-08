# Agent Pane Title Buttons + Git Identity

**Date:** 2026-04-15  
**Author:** AgentA  
**Status:** Proposed (superseded — see note below)

> **2026-08-07 audit note:** Superseded — references a "Forge" tab /
> `AgentCardSettingsPanel` no longer in code, replaced by the Identity/Armory
> consolidation (see `CLAUDE.md`'s "Not widgets" table). See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## 1. Title Buttons (Rename / Forge / Identity)

### Problem

When an agent is actively running in a pane, the only action button in the title bar is a back-arrow ("← back to picker"). The three agent management actions — Rename, Forge, Identity — are only accessible by going back to the picker, finding the agent card, and clicking there.

This is especially awkward when the agent is mid-session. The user should not have to navigate away just to rename or check identity.

### Behaviour

Three icon buttons appear in the pane title bar **only when an agent is loaded** (i.e. `block.meta.agentId` is set). They sit to the left of the existing back-arrow:

```
[✏ Rename]  [⚙ Forge]  [👤 Identity]  [← Back]
```

Clicking any of them opens the **same half-pane overlay** that the picker currently uses, but pre-positioned on the relevant panel for the current agent — so the user never sees the full agent list, just the action they asked for.

The overlay covers **the top half of the agent chat area** (same as the picker overlay), leaves the input area visible at the bottom, and is dismissed by clicking outside or pressing Escape.

### States

| Condition | Buttons shown |
|-----------|--------------|
| Picker visible (no agent loaded) | None — back-arrow also hidden |
| Agent loaded, idle | ✏ ⚙ 👤 ← |
| Agent loaded, turn active (Working…) | ✏ ⚙ 👤 ← (all enabled — management actions don't interrupt the turn) |

### Implementation plan

#### `agent-model.ts` — extend `endIconButtons`

`endIconButtons` currently returns `[backArrow]` when an agent is loaded. Extend it to return `[rename, forge, identity, backArrow]`.

Each button sets a signal that controls which panel the overlay opens to:

```typescript
// new signals in AgentViewModel
showOverlayTab = createSignal<"rename" | "forge" | "identity" | null>(null);

endIconButtons = () => {
    const agentId = this.blockAtom()?.meta?.["agentId"];
    if (!agentId) return [];
    return [
        {
            elemtype: "iconbutton",
            icon: "pencil",
            title: "Rename this agent",
            click: () => setShowOverlayTab("rename"),
        },
        {
            elemtype: "iconbutton",
            icon: "settings",
            title: "Configure in Forge",
            click: () => setShowOverlayTab("forge"),
        },
        {
            elemtype: "iconbutton",
            icon: "person",
            title: "Manage identity",
            click: () => setShowOverlayTab("identity"),
        },
        {
            elemtype: "iconbutton",
            icon: "arrow-left",
            title: "Back to agent picker",
            click: () => void this.backToPicker(),
        },
    ];
};
```

#### `agent-view.tsx` — overlay when `showOverlayTab` is set

Currently the picker is shown when `showPicker()` is true. Add a second overlay path: when `showOverlayTab()` is non-null, render a focused single-agent panel rather than the full picker list.

The overlay reuses `AgentCardSettingsPanel` (already exists for forge/identity tabs) and the rename input from `AgentCard`. No new components needed — just wire the existing panels with the agent from `blockMeta.agentId`.

```tsx
<Show when={showOverlayTab() != null}>
    <div class="agent-overlay agent-overlay-focused">
        <AgentFocusedPanel
            agent={currentAgent()}
            initialTab={showOverlayTab()!}
            onClose={() => setShowOverlayTab(null)}
        />
    </div>
</Show>
```

`AgentFocusedPanel` is a thin wrapper:
- `tab === "rename"` → shows the rename input (same as AgentCard inline rename)
- `tab === "forge"` → renders `AgentCardSettingsPanel` with `activeTab="forge"`
- `tab === "identity"` → renders `AgentCardSettingsPanel` with `activeTab="identity"`

#### SCSS

The overlay uses the same `.agent-overlay` styles that the picker uses. The focused variant:
- Width: full pane width
- Height: ~50% of chat area (same as picker)
- Position: absolute, top of chat area
- Background: same dark panel background
- `z-index` sits above chat, below the title bar

#### Dismiss behaviour
- Click outside the panel → `setShowOverlayTab(null)`
- Escape key → `setShowOverlayTab(null)`
- Rename committed (Enter / blur) → `setShowOverlayTab(null)` after RPC completes
- Forge/Identity changes → panel stays open (settings are persisted on each change, not on close)

---

## 2. Git Identity for In-Pane Agents

### Problem

Agents running inside AgentMux (AgentX, etc.) inherit the host machine's global git identity — `user.name` and `user.email`. On machines where this isn't set, every `git commit` either fails or uses an empty identity, producing `*** Please tell me who you are.` errors.

AgentMux already sets `AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_SLUG`, and per-agent `GH_CONFIG_DIR`. It does **not** set `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `GIT_COMMITTER_NAME`, or `GIT_COMMITTER_EMAIL`.

### Fix

Two layers, both required:

#### Layer 1 — Per-agent git env vars at launch

In `agent-model.ts`, when building the env vars for an agent launch, add:

```typescript
// Git identity — derived from the agent's Identity panel accounts,
// falling back to the agent slug.
const gitName = agent.name;                          // e.g. "AgentX"
const gitEmail = agentEmail(agent) ?? `${slug}@agents.local`;

envVars["GIT_AUTHOR_NAME"]    = gitName;
envVars["GIT_AUTHOR_EMAIL"]   = gitEmail;
envVars["GIT_COMMITTER_NAME"] = gitName;
envVars["GIT_COMMITTER_EMAIL"]= gitEmail;
```

`agentEmail(agent)` reads from `agent.accounts` — if the agent has a GitHub account attached whose email is known, use that. Otherwise fall back to `${slug}@agents.local` (a safe placeholder that satisfies git's format requirement without being a real address).

This covers the common case: git commits made by the agent in its working directory are attributed correctly without any user setup.

#### Layer 2 — Per-agent gitconfig in working directory

For repos that don't honour env vars (some git GUI tools, some hooks), also write a `.gitconfig` in the agent's working directory on first launch:

```
[user]
    name  = AgentX
    email = agentx@agents.local
```

Stored at `~/.agentmux/agents/<slug>/.gitconfig` and activated via:
```
envVars["GIT_CONFIG_GLOBAL"] = `~/.agentmux/agents/${slug}/.gitconfig`;
```

`GIT_CONFIG_GLOBAL` overrides `~/.gitconfig` for this process only — it does not affect the user's own git identity.

#### Where to implement

- `frontend/app/view/agent/agent-model.ts` → add env vars in the launch section (~line 273)
- `agentmux-srv/src/backend/forge/` (or equivalent) → write `.gitconfig` on agent workspace init

#### Identity panel integration (future)

When the user sets an email in the Identity panel for a GitHub account, propagate it to the per-agent gitconfig automatically. This is a follow-on; the `@agents.local` fallback is sufficient for now.

---

## Files to change

| File | Change |
|------|--------|
| `frontend/app/view/agent/agent-model.ts` | Add 3 icon buttons to `endIconButtons`; add `showOverlayTab` signal; add git env vars |
| `frontend/app/view/agent/agent-view.tsx` | Render `AgentFocusedPanel` overlay when `showOverlayTab` is set |
| `frontend/app/view/agent/components/` | New `AgentFocusedPanel.tsx` (thin wrapper around existing panels) |
| `frontend/app/view/agent/agent-view.scss` | `.agent-overlay-focused` positioning styles |
