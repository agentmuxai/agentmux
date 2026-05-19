# SPEC: Rename `Workflows` feature → `Drone`

**Date:** 2026-05-18
**Author:** AgentX
**Status:** Draft — pending RFC sign-off
**Supersedes:** Decision in [`specs/REPORT_DRONE_VS_WORKFLOW_ENGINE_2026_05_08.md`](../../specs/REPORT_DRONE_VS_WORKFLOW_ENGINE_2026_05_08.md) which chose "Workflows" over "Drone"

---

## TL;DR

Rename the recently-added Workflows feature (PR #755 Phase 1, PRs #831-848 Phase 1.5) to **Drone**. Better signals that the feature is an *automated* / *autonomous* DAG runner rather than a generic process-orchestration tool. Hard cut over 2-3 atomic PRs, ~4 days of effort.

---

## 1. Scope (what counts as "the feature")

The Workflows feature is the DAG executor widget pane:

| Layer | Surface |
|---|---|
| Widget registry | `defwidget@workflows` in `agentmux-srv/src/config/widgets.json` |
| Frontend view | `frontend/app/view/workflows/` (6 files: `workflows.tsx`, `workflows-view.tsx`, `workflows-model.ts`, `workflows-types.ts`, `workflows-view.scss`, `block-registry.ts`) |
| Frontend state | `frontend/app/store/workflow-run-state-store.ts` (+ test, + `workflow-run-state/` subdir with `reducer.ts`, `types.ts`, `index.ts`) |
| Backend module | `agentmux-srv/src/workflows/` (`mod.rs`, `types.rs`, `storage.rs`, `data_flow.rs`, `executor/engine.rs`, `executor/blocks/{agent,api,condition,response,variables}.rs`) |
| Backend handlers | `agentmux-srv/src/server/workflow_handlers.rs` |
| RPC / wire | `workflow_*` methods in `backend/rpc_types.rs`, SSE topic `workflowrun` in `websocket.rs` |
| Storage | `wstore` keys for workflow defs; migration entries in `backend/storage/migrations.rs` |
| Generated types | `frontend/types/gotypes.d.ts` (14 refs — auto-regenerated from Rust) |

**808 raw occurrences across 107 files**, but most of those are **not** the feature. See §3.

---

## 2. Naming taxonomy

| Concept | Old | New |
|---|---|---|
| The feature / widget | "Workflows" | "Drone" |
| A single saved DAG definition | "workflow" | "drone" |
| A single execution of a DAG | "workflow run" | "drone run" |
| A node in the DAG | "block" | **keep "block"** (orthogonal to feature naming) |
| The Agent block type | "Agent block" | "Agent block" (unchanged) |
| Widget key | `defwidget@workflows` | `defwidget@drone` |
| View key | `view: "workflows"` | `view: "drone"` |
| RPC namespace | `workflow_*` | `drone_*` |
| SSE topic | `workflowrun` | `dronerun` |
| URL/file paths | `workflows/` | `drone/` |
| Rust module | `agentmux_srv::workflows` | `agentmux_srv::drone` |

---

## 3. What NOT to rename (critical)

Of the 107 hits for `workflow`, the following are generic usage and must be untouched:

- **`.github/workflows/`** — GitHub Actions
- **Changesets workflow** — `.changesets/1778835927-devx-phase-2-adopt-changesets-workflow-rfc-857.md`, related docs in `CLAUDE.md`
- **Git workflow / release workflow** — `CLAUDE.md` § "Git Workflow", "Feature PR workflow", "Release PR workflow"
- **Auth flow / lifecycle workflow** — generic process language in specs
- **`docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md`** — talks about "workflow" generically
- **Anything in `specs/archive/`** — historical, immutable

A naive `sed -i s/workflow/drone/g` would break ~60% of the hits. The refactor needs symbol-aware tooling and per-file judgment, not regex.

---

## 4. Phased approach

### Phase 0 — Decision & RFC (½ day)

- Open RFC issue: *"Rename Workflows feature → Drone"*
- Body cites `REPORT_DRONE_VS_WORKFLOW_ENGINE_2026_05_08.md` and explains the reversal: feature is automated/autonomous, "drone" better signals that intent than the generic "workflow"
- Get sign-off before any code changes
- Coordinate with anyone on open `agenta/workflows-*` branches (currently `agenta/workflows-phase-1-5-types`) — rebase pain is real

### Phase 1 — Backend rename (1 day)

*Atomic PR. No semantic changes.*

1. `git mv agentmux-srv/src/workflows agentmux-srv/src/drone`
2. `git mv agentmux-srv/src/server/workflow_handlers.rs agentmux-srv/src/server/drone_handlers.rs`
3. Rename Rust types: `Workflow → Drone`, `WorkflowRun → DroneRun`, `WorkflowExecutor → DroneExecutor`, `WorkflowStorage → DroneStorage`, `WorkflowEvent → DroneEvent`
4. Rename RPC methods: `workflow_create → drone_create`, `workflow_run → drone_run`, etc.
5. Rename SSE topic: `workflowrun → dronerun`
6. Update `widgets.json`: key + view + label
7. Regenerate `gotypes.d.ts` (emitted from Rust types — confirm with build)
8. `cargo build --release -p agentmux-srv` must pass
9. `cargo test -p agentmux-srv` must pass

### Phase 2 — Frontend rename (1 day)

*Atomic PR, depends on Phase 1.*

1. `git mv frontend/app/view/workflows frontend/app/view/drone`
2. Inside, rename files: `workflows.tsx → drone.tsx`, `workflows-view.tsx → drone-view.tsx`, etc.
3. `git mv frontend/app/store/workflow-run-state frontend/app/store/drone-run-state`
4. `git mv frontend/app/store/workflow-run-state-store.ts frontend/app/store/drone-run-state-store.ts` (+ test)
5. Symbol rename: `WorkflowsView → DroneView`, `WorkflowsModel → DroneModel`, `useWorkflowRunStore → useDroneRunStore`, store action types
6. Update RPC call sites in `rpc-api.ts` to match Phase 1 RPC names
7. Update view registry / `block.tsx` / `blockutil.tsx` references
8. CSS class rename: `.workflows-* → .drone-*` in `drone-view.scss`
9. UI strings: "Workflows" label, empty-state copy, tooltips
10. `npm run build:dev` + `task dev` smoke test (open the pane, run a sample DAG, watch SSE)

### Phase 3 — Storage migration (½ day)

*Same PR as Phase 1 ideally, but called out separately because of the risk.*

Inspect `agentmux-srv/src/backend/storage/migrations.rs` and `wstore.rs` for:

- SQLite table names containing `workflow`
- wstore key prefixes containing `workflow`
- Persisted JSON shape fields containing `workflow`

For each:

- If the data is **ephemeral / per-session**: rename in place, no migration (existing data discarded on restart — acceptable for a dev-only feature)
- If the data is **persisted across launches**: write a migration entry that renames keys/tables on first boot. The pattern is already in `migrations.rs` (15 hits) — follow the existing convention.

**Decision needed:** is current workflow state preserve-worthy or disposable? Feature is days old; lean disposable but confirm.

### Phase 4 — Docs & specs (½ day)

*Separate PR, low-risk.*

1. `CLAUDE.md` — widgets table row, any narrative mentions
2. `README.md`, `BUILD.md`, `VERSION_HISTORY.md` — search-and-review-each
3. `docs/specs/` — rename **active** specs that title-reference Workflows. Leave historical ones (e.g., `REPORT_DRONE_VS_WORKFLOW_ENGINE_2026_05_08.md`) as historical record; add a header note: *"Decision reversed 2026-05-18 — see SPEC_RENAME_WORKFLOWS_TO_DRONE_2026_05_18.md"*
4. Update `SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md` (42 refs — most are the feature)
5. Add `.changesets/<ts>-refactor-rename-workflows-to-drone.md`

### Phase 5 — Cleanup & lint guard (½ day)

- Grep for leftover `Workflow`/`workflow` in renamed surfaces — expect 0 in `src/drone/`, `view/drone/`, `store/drone-run-state/`
- Add a small CI grep guard (optional): if anyone reintroduces `view/workflows/` or `workflow_handlers.rs`, fail. Probably overkill.

---

## 5. Rollout posture

**Recommendation: hard cut, atomic over 2-3 PRs.**

- Feature is days old, has no external users, no integrations depend on the widget key, no persisted user data we need to preserve
- Aliases / back-compat add code-debt for zero benefit
- Coordinate with anyone holding `agenta/workflows-*` branches — they rebase against renamed paths

**Exception:** if there's a saved layout file in `~/.agentmux/` referencing `defwidget@workflows`, the layout migration in Phase 3 should map it. Otherwise users with the pane open lose it on first launch after upgrade.

---

## 6. Risk register

| Risk | Mitigation |
|---|---|
| Generic "workflow" usages get caught in rename | Symbol-level renames (not regex), reviewer scrutiny in §3 list |
| `gotypes.d.ts` drift | Regenerate from Rust source; verify diff matches expectations |
| In-flight branches conflict | Coordinate ahead via RFC issue; rebase guidance in PR description |
| RPC wire breakage between launcher / srv / frontend | All three components ship in lockstep (same binary, same package), so no cross-version compat needed |
| Persisted workflow data lost | §3 migration entry; or accept loss if feature is dev-only |
| Reviewers confused by reversal of 2026-05-08 decision | RFC body explains rationale; historical report annotated, not deleted |
| Branding inconsistency with the literal word "workflow" appearing in user-facing copy elsewhere | Final pass over UI strings, tooltips, error messages |

---

## 7. Validation checklist

- [ ] `cargo build --release` green
- [ ] `cargo test --workspace` green
- [ ] `npm run build:prod` green
- [ ] `task package` produces working binary
- [ ] Open the "Drone" pane from More dropdown
- [ ] Create a sample DAG, run it, verify SSE streams `dronerun` events
- [ ] Restart app, verify saved drones reload (or are migrated)
- [ ] Grep `Workflow|workflow` in renamed dirs returns 0
- [ ] Generic `workflow` usages (CI, changesets, git workflow docs) preserved

---

## 8. Effort estimate

| Phase | Effort |
|---|---|
| 0. RFC | ½ day |
| 1. Backend | 1 day |
| 2. Frontend | 1 day |
| 3. Storage | ½ day |
| 4. Docs | ½ day |
| 5. Cleanup | ½ day |
| **Total** | **~4 days** |

---

## 9. Rationale for reversing the 2026-05-08 decision

The original report (`REPORT_DRONE_VS_WORKFLOW_ENGINE_2026_05_08.md`) chose "Workflows" because:

- It modeled the feature on Sim (simstudioai/sim) which uses "workflow" terminology
- "Drone" in PR #256 was a narrower concept (scheduled Forge agent)
- Wanted a general DAG executor, not a "scheduled agent" tool

This rename reverses that for these reasons:

1. **User mental model:** "Workflows" is generic and overloaded (Git workflows, CI workflows, business workflows). "Drone" signals *autonomous execution* — the feature *runs by itself* on triggers, not just a static pipeline you invoke manually.
2. **Differentiation:** Agent panes are interactive; Drone panes are unattended. The naming should reflect that contrast.
3. **Brand cohesion:** AgentMux's pane vocabulary (Agent, Browser, Terminal, Sysinfo, Swarm) is short, concrete, evocative. "Workflows" is the odd one out.
4. **Sim parity is not a requirement:** the engine architecture stays Sim-modeled (DAG, blocks, triggers); only the user-facing name changes.

The 2026-05-08 report's *technical* recommendations (solid-flow canvas, Rust DAG engine, block taxonomy) remain in force. Only the *name* is being revised now that the feature is built and we can evaluate how it actually reads.
