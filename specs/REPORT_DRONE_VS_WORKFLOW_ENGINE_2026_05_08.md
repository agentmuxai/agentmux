# Report: PR #256 (Drone) vs. Workflow Engine Spec — best way forward

**Date:** 2026-05-08
**Author:** review of PR #256 + agenty-workspace workflow specs
**Sources:**
- PR: <https://github.com/agentmuxai/agentmux/pull/256> ("Drone pane — Phase 1 node-graph canvas for automated agents")
- Spec: `~/.claw/agenty-workspace/agentmux-workflow-engine-plan.md` (2026-05-02)
- Spec: `~/.claw/agenty-workspace/sim-agentmux-integration.md` (2026-05-02)
- No GitHub issue tracks the workflow-engine pivot yet — the integration doc planned to file one ("RFC: Sim Workflow Pane Integration") but it was never opened.

---

## TL;DR

PR #256 (Drone) and the workflow-engine plan target **overlapping but materially different** features. Drone is a narrower "schedule + chain agents" orchestrator. The workflow-engine plan is a general DAG executor modeled after [Sim (simstudioai/sim)](https://github.com/simstudioai/sim) — 28.3k-star agent workflow builder. PR #256 is **APPROVED but DIRTY** (won't apply to current main; predates the `agentmuxsrv-rs/` → `agentmux-srv/` rename and Tauri removal).

**Recommendation: rename + reframe.** Close PR #256, file a new RFC issue lifting the workflow-engine plan, salvage ~30 % of the frontend (canvas + edit-panel scaffolding, status semantics) into a new `frontend/app/view/workflows/` tree backed by `solid-flow`. Drone's cron + dependency triggers become *Phase 2* of the workflow engine (the "trigger" surface), not a separate widget.

---

## 1. The famous library

| | |
|---|---|
| **Name** | Sim (simstudioai/sim) |
| **Stars** | 28.3k · Apache 2.0 · multiple commits/day as of May 2026 |
| **What it is** | Visual workflow builder for AI agents — ReactFlow canvas, 1,000+ block integrations, vector DB, copilot, multi-agent orchestration |
| **AgentMux's reading** | "Central intelligence layer for agentic workforces" — the model AgentMux wants to replicate natively for its widget pane |

Two docs in `~/.claw/agenty-workspace/` capture the thinking:

1. **`sim-agentmux-integration.md`** — evaluates 4 integration options (iframe, headless API, component lib, fork). Recommends *A+B hybrid* (iframe probe → headless API pane).
2. **`agentmux-workflow-engine-plan.md`** — pivots to **Option D (native fork)**: build the engine in SolidJS + Rust, with `solid-flow` for the canvas. This is the spec that supersedes PR #256's design.

---

## 2. Side-by-side: Drone (PR #256) vs Workflow Engine (spec)

| Dimension | PR #256 Drone | Workflow Engine plan |
|-----------|---------------|----------------------|
| **Mental model** | Automated **agents** triggered by cron/event/dependency | General **DAG of blocks**: agents are one of many block types |
| **Canvas library** | Custom DOM-based pan/zoom in `drone-view.tsx` (~750 LOC of CSS) | `solid-flow` (MIT, native SolidJS, signal-driven) |
| **Block types** | 1: a Drone (which wraps a Forge agent) | 6 in P1 (Agent, Condition, API, Function, Response, Variables) + 7 P2 (Loop, Parallel, Router, Workflow, Webhook, Wait, Evaluator) |
| **Trigger surface** | cron, event, dependency, webhook, manual, watchdog | Manual, REST API, Webhook, Schedule (cron) — same surface, but expressed as deployment triggers, not block edges |
| **Execution engine** | Frontend-only mock; Phase 2 plans `DroneScheduler` in Rust | Full topological-sort DAG runner in Rust (`executor/engine.rs`); SSE streaming; per-block status; data-flow propagation |
| **Variable resolution** | Implicit per-drone task prompt | First-class: `<block.id.output>`, `<var.name>`, `<loop.index>` resolver chain |
| **State store** | `drone-model.ts` (SolidJS signals, ad-hoc) | `flowStore` + `executionStore` + `workflowStore` — three separate stores with clear boundaries |
| **Versioning** | Out of scope | Draft vs deployed; full snapshots; promote/rollback |
| **Phase 1 effort** | "Phase 1 frontend scaffold" (the 2.5 k-line PR) | 10 weeks (Canvas → Palette/Inspector → Rust engine → Run panel → polish) |
| **Backend layout** | `agentmux-srv/src/drone/` (planned, not built) | `agentmux-srv/src/{handlers/workflows.rs, executor/, storage/{workflows,versions}.rs}` |
| **Widget key** | `defwidget@drone` | `defwidget@workflows` |

Where they overlap:

- Both render a node-graph canvas with pan/zoom, drag, edge drawing.
- Both bottom-drawer a run log; both right-panel an edit panel.
- Both want SSE streaming of run output back to the canvas.
- Both fold cron + event triggers into the model.

Where they diverge:

- **Scope:** Drone wraps a single primitive ("Drone = scheduled Forge agent"); Workflow has many primitives composed through edges + a real interpreter.
- **Execution:** Drone has none yet; Workflow has a fully-specified Rust DAG runner with parallel layers, variable scope, and error containment.
- **Canvas:** Drone re-implements the hard parts (pan/zoom, drag, edge math) in custom CSS/JS; Workflow delegates to `solid-flow`. The Workflow spec explicitly evaluated and rejected the custom approach.
- **State machine:** Drone has `idle → queued → running → success | failed | retrying` per drone run. Workflow has per-block `pending | running | done | error | skipped`. Different granularity — a Drone run ≈ executing a single workflow block.

---

## 3. PR #256's current standing

| Field | Status |
|-------|--------|
| Review state | **Approved** (ReAgent LGTM after 2 round-trips on minor issues) |
| Mergeability | **Dirty** — touches `agentmuxsrv-rs/` (renamed to `agentmux-srv/`), `src-tauri/`, `wsh-rs/Cargo.toml` (all removed/renamed since) |
| AgentA-asaf triage 2026-05-07 | "Sizable feature; rebase is non-trivial. Recommend either dedicating a session to rebase + re-test, or close and reopen as Phase 1.5 with current architecture." |
| Frontend salvageability | **High** for `drone-types.ts`, `drone-utils.ts` (cron preview, status helpers), `drone-edit-panel.tsx` shape, `drone-run-log.tsx` shape |
| Frontend salvageability | **Low** for `drone-view.tsx` + `drone-view.css` — the workflow-engine plan says *use solid-flow*, which makes ~750 lines of custom canvas CSS dead on arrival |
| Backend salvageability | **None** — `agentmuxsrv-rs/src/drone/` was never written; the spec entries pointed at a directory that was renamed before any code landed |

---

## 4. Three options for the way forward

### Option 1 — Rebase + ship Drone as-is (Phase 1.5)
Get PR #256 over the line on the current architecture (drop Tauri/wsh refs, port `agentmuxsrv-rs/` paths). Defer the workflow-engine pivot to a follow-up.

| Pro | Con |
|-----|-----|
| Already approved; closest to the finish line | Locks in the narrower "Drone wraps an agent" mental model |
| Cron + dependency triggers ship sooner | The 750-LOC custom canvas becomes legacy you'll need to rip out for `solid-flow` |
| Existing review feedback (canvas keyframes dead-code, etc.) is already triaged | When the workflow engine arrives, two overlapping panes confuse users (Drone + Workflows) |

**Estimated effort:** 2-3 days rebase + re-test.

### Option 2 — Close PR #256, start fresh from workflow-engine plan (recommended)
File a new RFC issue from `agentmux-workflow-engine-plan.md`. Build `defwidget@workflows` on `solid-flow`, with the 6-block MVP. Drone's trigger types (cron, event, dependency) become the **trigger configuration** of a deployed workflow, not a separate widget.

| Pro | Con |
|-----|-----|
| Aligns with where the thinking went after PR #256 | 10-week MVP vs days for rebase |
| One pane instead of two; clearer mental model for users | Loses the approved review on PR #256 (need fresh review on the new code) |
| `solid-flow` removes ~750 LOC of custom canvas you'd otherwise carry | Up-front canvas-library decision; if `solid-flow` disappoints you backtrack |
| Rust DAG executor is a strict superset of any "drone runner" — no rewrite later | More work before any user-visible feature |

**Estimated effort:** Phase 1 = 10 weeks (per the spec).

### Option 3 — Hybrid: rename PR #256 to "Workflows MVP", retrofit the spec into it
Keep the PR's approved frontend scaffolding, rebase onto current main, but rename `drone` → `workflows`, replace the custom canvas with `solid-flow`, and grow the block registry from "1 block (Drone)" to "6 blocks" in subsequent PRs.

| Pro | Con |
|-----|-----|
| Salvages the approved scaffolding (edit panel, run log, status semantics, cron preview) | High risk of "neither / nor" — neither a clean workflow engine nor a shipped Drone |
| One pane, one mental model | Breaks PR #256's review history; reviewers will re-review the whole thing anyway |
| Smaller incremental PRs after rebase | Renaming-during-rebase is messy in git |

**Estimated effort:** ~1 week rebase + rename + canvas swap, then ~8-9 weeks of follow-up PRs.

---

## 5. Recommendation

**Option 2 with selective salvage.** Concretely:

1. **Today:** file `agentmuxai/agentmux` issue *"RFC: Workflows pane (Sim-modeled DAG executor)"* — paste in `agentmux-workflow-engine-plan.md` as the body, link to PR #256 as superseded.
2. **This week:** comment on PR #256 with a link to the RFC and close it. Note explicitly which files we plan to lift (`drone-types.ts` shape, `drone-utils.ts` cron-preview helper, edit-panel/run-log component shapes, `defwidget@drone` removed from widgets.json plan).
3. **Next iteration:** open the PR for **Workflows Phase 1, Week 1-2** from the build plan: `solid-flow` + `flowStore` + 6 block components + widget registration. That's the first reviewable chunk; subsequent weeks are independent PRs that can ship in sequence.
4. **Do not** carry Drone's custom canvas forward — it duplicates what `solid-flow` already does and inflates the diff. The workflow-engine spec rejected this trade explicitly.

Cron / dependency triggers (the Drone differentiator) re-emerge in **Workflows Phase 2** (`tokio-cron-scheduler`, webhook receivers, dependency edges between *deployed workflows*). No feature is lost; the surface is just unified under one widget.

---

## 6. Open questions before opening the RFC

1. **Solid-flow vs `@dschz/solid-flow`** — pick one. The spec lists `solid-flow` as primary, the `@dschz` fork as backup. A 30-min spike on each + a decision note locks this in.
2. **Function block sandbox** — the spec says "JS/Python in sandbox" but doesn't pick the sandbox. `isolated-vm` (Sim's choice) requires Node-API; alternatives: `quickjs-rs`, separate subprocess. Affects Rust deps.
3. **Storage** — workflow JSON in SQLite (`storage/workflows.rs`) or in the existing wstore? The spec defaults to SQLite; consistency with `db_forge_agents` argues for wstore. **Recommendation: wstore for workflows definitions, SQLite for execution-history time-series** (mirrors the same pattern proposed in PR #256's spec for Drone runs).
4. **Block ↔ Forge agent overlap** — the Workflows "Agent block" and Forge agents both wrap LLM calls. Is the Agent block a *reference* to a Forge agent definition, or its own definition? The spec is silent. **Recommendation: reference. Keeps Forge as the single authority for agent identity + tools.**
5. **Sim `<var>` interpolation collisions** — Sim's `<block.id.output>` syntax conflicts with markdown-style `<...>` content the agent might emit. The spec hand-waves this; need a quoting / escape rule before implementation.

---

## 7. What this looks like in main repo terms

| Today | Becomes |
|-------|---------|
| Open: PR #256 (Drone) — approved, dirty | Closed with link to new RFC |
| `~/.claw/agenty-workspace/agentmux-workflow-engine-plan.md` | New issue body; copied into `specs/SPEC_WORKFLOWS_PANE_2026_05_08.md` |
| `~/.claw/agenty-workspace/sim-agentmux-integration.md` | Linked from RFC as background; archived (analysis is done) |
| `defwidget@drone` (planned) | `defwidget@workflows` (registered in `agentmux-srv/src/config/widgets.json`) |
| `frontend/app/view/drone/` (PR #256 branch) | `frontend/app/view/workflows/` (new branch, `solid-flow`-based) |
| `agentmux-srv/src/drone/` (planned, never built) | `agentmux-srv/src/{executor/, storage/workflows.rs, handlers/workflows.rs}` |

---

*End of report.*
