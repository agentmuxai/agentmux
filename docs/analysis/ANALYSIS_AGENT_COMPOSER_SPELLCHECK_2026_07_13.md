# Analysis: agent conversation composer shows the native red-squiggly spellcheck underline (2026-07-13)

**Author:** Agent2
**Status:** Root cause identified, fix implemented.
**Reported by:** user — "in the conversation input in agent panes, we don't need the red squiggly line spell check."

## Symptom

Typing a message into the agent pane's conversation composer (the textarea at the bottom of the pane, where the user talks to the agent) shows Chromium's native spellcheck red-squiggly underline under words it doesn't recognize — code identifiers, file paths, tool/CLI names, shorthand, etc. almost always trigger it, so in practice it's mostly noise.

## Root cause

The composer `<textarea>` (`frontend/app/view/agent/components/AgentFooter.tsx`, ~line 794) had no `spellcheck` attribute set. `<textarea>` defaults to `spellcheck="true"` (inherited from the document, which AgentMux — a Chromium/CEF app — doesn't override globally), so Chromium's built-in spellchecker runs on it and paints the red squiggly underline.

This is a straightforward omission, not a deliberate choice: `spellcheck={false}` is already an established, consistently-applied convention across the rest of the codebase for exactly this reason. It's already set on every other significant text input in the app:

- `frontend/app/modals/command-palette.tsx`
- `frontend/app/view/agent/components/AgentMcpModal.tsx`
- `frontend/app/view/agent/components/AgentNativeMemoryModal.tsx`
- `frontend/app/view/agent/components/AgentSkillsModal.tsx`
- `frontend/app/view/agent-def/components/AgentSkillForm.tsx`
- `frontend/app/view/agent-def/components/ContentEditor.tsx`
- `frontend/app/view/brain/global-brain-manager.tsx`
- `frontend/app/view/identity/identity-view.tsx`
- `frontend/app/view/mcp/mcp-manager.tsx`
- `frontend/app/view/skill/skill-manager.tsx`

The agent pane's conversation composer — by a wide margin the most-used text input in the entire app — was the one conspicuous gap in that pattern.

## Fix

Added `spellcheck={false}` to the composer `<textarea>` in `AgentFooter.tsx`, matching the existing convention exactly (same prop, same literal value, same SolidJS boolean-attribute form used everywhere else above).

No other changes needed: the `<textarea>` has no companion "spellcheck toggle" setting anywhere in the app, so there's no configuration surface to also update, and no other input in the agent pane's conversation area (the composer is the only free-text entry point in that zone — autocomplete/slash-command selection is keyboard/mouse-driven, not typed prose) needed the same treatment.
