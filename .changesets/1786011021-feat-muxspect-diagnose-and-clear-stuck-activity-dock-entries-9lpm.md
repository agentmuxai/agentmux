---
type: minor
---

feat(muxspect): diagnose and clear stuck Activity Dock entries (dock, dock clear)

`muxspect dock <block_id>` reads a live snapshot of an Agent pane's ToolNode statuses and flags entries that look stuck (e.g. a tool call rejected by the outer CLI harness before it ever ran, which never receives a terminating event and stays "running" indefinitely). `muxspect dock clear <block_id> <node_id>` force-clears one, live, in whatever renderer has that block open — no pane reload needed. muxspect's first mutating command; every other command remains read-only. See docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md.
