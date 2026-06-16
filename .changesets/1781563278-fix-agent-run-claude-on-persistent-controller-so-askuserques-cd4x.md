---
type: minor
---

feat(agent): answer AskUserQuestion via the Agent SDK control protocol

Claude now runs on the persistent controller with `--permission-prompt-tool stdio`
and the sidecar answers the CLI's `can_use_tool` control_request with a
control_response (`updatedInput.answers`). This is the only mechanism the CLI
accepts for AskUserQuestion — delivering a tool_result on stdin does not work
(the CLI auto-rejects it within the turn). Ordinary tools are auto-allowed to
preserve current behavior. Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
