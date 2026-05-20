---
type: patch
---

refactor(a2): retire the "forge" vocabulary from the IPC wire + frontend

Follow-up A2 to the storage de-forge (PR #934). That PR renamed the Rust
storage layer but deliberately left the IPC wire command strings and the
whole frontend `forge` view untouched (decision A1). This completes the job
— "forge" is now gone from every layer a developer reasons about.

The IPC wire is internal (CEF frontend ↔ srv, shipped together), so the
command strings are renamed outright with no compat shim; srv + frontend
land atomically.

- **Wire commands** — `listforgeagents` → `listagents`, `createforgeagent`
  → `createagent`, `getforgecontent` → `getagentcontent`,
  `listforgeskills` → `listagentskills`, `appendforgehistory` →
  `appendagenthistory`, `importforgefromclaw` → `importagentfromclaw`,
  `reseedforgeagents` → `reseedagents`, etc. (18 commands). The
  `COMMAND_*_FORGE_*` constants rename to match.
- **Frontend view** — `frontend/app/view/forge/` → `view/agent-def/`;
  `forge-model.ts` → `agent-def-model.ts`, `forge-constants.ts` →
  `agent-def-constants.ts`; `ForgeViewModel` → `AgentDefViewModel`; the 9
  `Forge*` components → `AgentDef*` / `AgentSkill*` / `AgentContent*` /
  `AgentHistory*`.
- **Types** — `gotypes.d.ts` + `rpc-api.ts` updated: `ForgeAgent` →
  `AgentDefinition`, `ForgeContent` → `AgentContent`, `ForgeSkill` →
  `AgentSkill`, `ForgeHistory` → `AgentHistory`, and the `Command*Forge*Data`
  types, matching the Rust struct names from #934.
- **Settings/overlay tab** — the internal `"forge"` tab enum value →
  `"agent"`. The `block.tsx` `view: "forge"` → `"agent"` migration shim is
  kept (back-compat for already-persisted blocks).
- `forge-seed.json` → `agent-seed.json`; `seed_forge_agents` →
  `seed_agents`; `default_forge_icon` → `default_agent_icon`.

Out of scope (follow-up): the `forge-*` CSS class names (~74) are an
internal styling layer — renaming them risks silent visual regressions the
compiler can't catch, so they're left for a dedicated cosmetic sweep.

Verified: `agentmux-srv` builds clean; frontend vite build clean; 3,270
frontend tests pass.
