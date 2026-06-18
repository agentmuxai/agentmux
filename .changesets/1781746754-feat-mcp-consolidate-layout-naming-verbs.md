---
type: minor
---

feat(mcp): consolidate layout/introspection + naming verbs — `Layout(query)` replaces GetLayout/ListWindows/ListWorkspaces/ListTabs and `SetName(target,name)` replaces SetWindowName/SetTabName/SetPaneTitle/SetWorkspaceName (17→11 MCP tools). WhoAmI and the SetActiveTab/NewTab/FocusWindow navigation verbs are unchanged; every REST endpoint and capability is preserved. Cuts the per-turn MCP tool-definition footprint (see agent-pane latency report §3) while keeping the surface discoverable.
