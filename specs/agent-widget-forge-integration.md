# Spec: Agent Widget — Forge Integration

## Status: DRAFT
**Date:** 2026-03-10
**Supersedes:** specs/agent-pane-terminal-switch.md, specs/responsive-agent-pane.md

---

## Summary

Replace the current 6-button provider picker (Raw + Styled × Claude/Codex/Gemini)
with a Forge-connected agent selector. When the user opens an Agent pane, they see
a list of agents they've created in the Forge. Selecting one launches it directly
into presentation view. Raw mode is removed entirely.

---

## Current State (to be removed)

```
Agent pane opens
  └─ AgentProviderPicker
       ├─ "Raw" group
       │    ├─ [Claude]  → connectWithProvider() → view:"term", cmd:"claude"
       │    ├─ [Codex]   → connectWithProvider() → view:"term", cmd:"codex"
       │    └─ [Gemini]  → connectWithProvider() → view:"term", cmd:"gemini"
       └─ "Styled" group
            ├─ [Claude]  → connectStyled()       → agentMode:"styled"
            ├─ [Codex]   → connectStyled()       → agentMode:"styled"
            └─ [Gemini]  → connectStyled()       → agentMode:"styled"
```

Problems:
- 6 buttons is too many choices for a first-time experience
- Raw mode (terminal passthrough) was a POC — no persistent conversation, no structured output
- Provider selection is too low-level; users think in terms of agents they've configured, not raw CLIs
- Doesn't integrate with the Forge where users define agents

---

## New Design

```
Agent pane opens
  └─ AgentPicker (Forge-connected)
       ├─ [MyAgent1]  ─────────────────────────────────────────┐
       ├─ [MyAgent2]                                           │ launch → AgentPresentationView
       ├─ [MyAgent3]                                           │
       └─ [empty state] "No agents yet — create one in Forge" ─┘
```

### Empty State

When the user has no agents in the Forge:

```
┌──────────────────────────────────────────┐
│                                          │
│         No agents configured             │
│                                          │
│    [ + Create an agent in the Forge ]    │
│                                          │
└──────────────────────────────────────────┘
```

The button opens the Forge (mechanism TBD — new pane, tab, or modal).

### Agents Exist

Each agent is a button/card:

```
┌──────────────────────────────────────────┐
│  ⚡ My Claude Coder                      │
│  ✨ Design Reviewer                      │
│  🤖 Gemini Researcher                    │
│                                          │
│  [ + New agent ]   (links to Forge)      │
└──────────────────────────────────────────┘
```

- Clicking a card immediately launches in presentation view
- No confirmation, no mode selection — single click to go
- "+ New agent" link always visible so users can add more without hunting

---

## Presentation View (formerly "Styled View")

After selecting an agent, the pane switches to `AgentPresentationView`:

- Full conversation rendering (markdown, tool blocks, diffs, terminal output)
- Footer input for sending messages
- No toggle back to "Raw" — that mode no longer exists
- Agent name + icon shown in pane header

The existing `AgentStyledSession` component becomes `AgentPresentationView` with
the provider wiring replaced by agent-config wiring.

---

## Agent Definition (Forge schema)

Each Forge agent needs to expose at minimum:

| Field | Type | Purpose |
|-------|------|---------|
| `id` | string | Unique identifier |
| `name` | string | Display name in picker |
| `icon` | string | Emoji or icon ID |
| `description` | string (optional) | Subtitle in card |
| `output_format` | string | `"claude-stream-json"` \| `"gemini-stream"` \| etc. |
| `cli_command` | string | CLI to spawn (e.g. `"claude"`, `"gemini"`) |
| `cli_args` | string[] | Additional args passed to CLI |

The agent widget reads this config to know how to spawn the CLI and parse its
output. The output_format drives which translator/parser is used — same pipeline
as today, just configured on the agent rather than hard-coded to provider buttons.

---

## What Gets Removed

| Item | Why |
|------|-----|
| `AgentProviderPicker` component | Replaced by `AgentPicker` |
| `ProviderButton` component | Replaced by agent cards |
| `providers/` directory (claude-translator, codex-translator, gemini-translator, translator-factory) | Provider selection is gone; output format is per-agent config |
| `connectWithProvider()` in agent-model.ts | Raw mode removed |
| Raw mode `view:"term"` launch path | Raw was POC |
| `agentMode` / `agentProvider` meta keys | Replaced by `agentId` (Forge ID) |
| "Raw" / "Styled" mode labels and CSS classes | No longer meaningful |
| specs/responsive-agent-pane.md (the 6-button responsive layout) | Picker is entirely new |
| PRs #82, #85 (styled view pipeline fixes) | Likely superseded — evaluate after Forge integration lands |

## What Gets Kept / Reused

| Item | Status |
|------|--------|
| `AgentStyledSession` → renamed `AgentPresentationView` | Keep, rewire to agent config |
| `useAgentStream` | Keep — drives the stream parser regardless of agent |
| `stream-parser.ts` | Keep — parses provider-specific stream formats |
| `types.ts`, `state.ts` | Keep — document model and streaming state |
| `AgentDocumentView`, `AgentFooter`, all message block components | Keep — unchanged |
| `TerminalOutputBlock`, `AgentMessageBlock`, `DiffViewer`, etc. | Keep |
| `bootstrap.ts` | Keep — bootstrap output surfacing |

---

## Open Questions

1. **Forge API** — How does the agent widget query the Forge for the user's agent list?
   - Rust backend WS command (`ListAgentsCommand`)?
   - Atom backed by a WPS event subscription?
   - Needs Forge spec to define the data contract.

2. **Opening the Forge** — What's the Forge surface?
   - A dedicated view type (`view: "forge"`)?
   - A settings panel?
   - This spec defers to the Forge spec.

3. **No agents at first launch** — Is the Forge bundled and always available, or
   does it require an account/connection? The empty state CTA needs to handle both
   cases.

4. **Live updates** — If the user creates a new agent in the Forge while an Agent
   pane is open, should the picker refresh? Likely yes via WPS subscription.

5. **Multi-instance** — Can the same Forge agent be open in multiple panes
   simultaneously? Probably yes; each pane gets its own session ID.

6. **PR disposition** — PRs #82 and #85 contain pipeline fixes (text fragmentation,
   ANSI stripping, bootstrap output) that may still be relevant for the new
   presentation view. Evaluate once Forge integration is scoped.

---

## Implementation Phases

### Phase 1 — Picker shell (no Forge backend yet)
- Remove Raw mode and the 6-button layout
- Add `AgentPicker` with hardcoded mock agents (same Claude/Gemini/Codex, but
  presented as user agents with names)
- Clicking any card goes straight to presentation view
- Empty state UI with disabled "Create in Forge" CTA

### Phase 2 — Forge API
- Define `ListAgentsCommand` (or equivalent) in agentmuxsrv-rs
- Wire `AgentPicker` to live Forge data
- Enable "Create in Forge" navigation

### Phase 3 — Agent config drives output format
- Replace hardcoded translator selection with agent config's `output_format` field
- `providers/` directory removed

---

## Files Changed (estimated Phase 1)

| File | Change |
|------|--------|
| `frontend/app/view/agent/agent-view.tsx` | Remove `AgentProviderPicker`, `ProviderButton`, add `AgentPicker`; rename `AgentStyledSession` → `AgentPresentationView` |
| `frontend/app/view/agent/agent-view.scss` | Remove Raw/Styled mode groups; add agent card styles + empty state |
| `frontend/app/view/agent/agent-model.ts` | Remove `connectWithProvider()`, `connectStyled()` → `launchAgent(agentId)` |
| `frontend/app/view/agent/state.ts` | Replace `agentProvider`/`agentMode` meta with `agentId` |
| `frontend/app/view/agent/providers/index.ts` | Phase 1: keep temporarily; Phase 3: remove |
