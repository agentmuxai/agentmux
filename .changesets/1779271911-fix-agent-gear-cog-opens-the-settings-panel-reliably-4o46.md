---
type: patch
---

fix(agent): gear/cog opens the settings panel reliably

Clicking the ⚙ gear in an agent pane header did nothing. The gear flips an
overlay-tab signal, but the panel was rendered behind
`<Show when={showOverlayTab() != null && currentAgent() != null}>`.

`currentAgent()` resolves the pane's `agentId` block-meta against the
`db_agent_definitions` list. That only matches for panes launched via
`launchAgentDefinition` (the AgentPicker → launch-modal path). It is `null`
for provider quick-launch panes (`launchAgent` writes a *provider* id), for
a pane whose definition was deleted, and during the async window before /
if `ListAgentDefinitionsCommand` resolves. In every one of those cases the
gear silently no-op'd — click, nothing.

Fix: gate the overlay only on `showOverlayTab() != null` and pass
`currentAgent()` (possibly `undefined`) straight through. The panel chain
already handles a missing definition — `AgentCardSettingsPanel.agent` is
typed `AgentDefinition | undefined` (create-mode), and the Identity tab has
a "save the agent first" fallback. `AgentFocusedPanel`'s `agent` prop is
widened to `AgentDefinition | undefined` to match what it forwards.
