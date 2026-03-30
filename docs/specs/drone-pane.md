# Spec: Drone Pane — Automated Agent Orchestration View

**Date:** 2026-03-30
**Author:** Agent1
**Status:** Draft — pre-implementation
**View type:** `drone`
**Related:** `agent-model.ts`, `forge/`, `swarm/`, PR #253 (CEF)

---

## Overview

The Drone pane is a new AgentMux view type that lets users define, schedule, visualize,
and monitor automated agents. Unlike the Agent pane (interactive, human-in-the-loop),
a Drone runs autonomously — triggered by cron schedules, external events, or other drones
completing — with no human input expected during execution.

The UI is a **node graph canvas** where each node is a Drone, and edges represent
triggers ("when A completes, fire B") or data flow. The graph is the primary mental model:
drones are not a list of jobs, they are an orchestration topology.

---

## Problem Statement

Today in AgentMux, agents are interactive. Every Agent pane expects a human to send
messages and review output. There is no way to:

- Run an agent on a cron schedule (e.g., "run this every morning at 7am")
- Chain agents so one fires after another completes
- React to external events (a GitHub webhook, a file appearing, a failed process)
- Monitor a fleet of background agents without opening each pane
- Retry a failed agent run automatically

The Drone pane solves all of this. It extends the existing Forge agent definition system
(which already captures provider, soul, agentmd, skills, env) with a scheduling and
orchestration layer.

---

## Core Concepts

### Drone
A Drone is an automated agent run definition. It has:
- A **Forge agent** as its execution unit (or an inline agent definition)
- One or more **triggers** (cron, event, dependency, manual)
- A **run policy** (retry count, timeout, concurrency limit)
- A **task** (the prompt/instruction sent to the agent when it fires)
- A **state machine** per run: `idle → queued → running → success | failed | retrying`

### Trigger Types
| Type | Description | Example |
|------|-------------|---------|
| `cron` | Time-based schedule (cron expression) | `0 7 * * *` — daily 7am |
| `event` | Named event from AgentBus or internal bus | `pr.opened`, `deploy.failed` |
| `dependency` | Another drone completed (success or failure) | "after ReportDrone succeeds" |
| `webhook` | HTTP POST to a local endpoint | Inbound GitHub webhook |
| `manual` | Only runs when explicitly triggered by user | On-demand drones |
| `watchdog` | Another drone has been idle longer than N minutes | Health check escalation |

A single drone can have multiple triggers (any one fires it).

### Run History
Every drone execution is a **run record** with:
- Trigger type and source
- Start/end timestamps
- Exit state (success, failed, cancelled, timed_out)
- Agent output (scrollback or summary)
- Retry attempt number

---

## UI Design

### Layout Overview

```
┌─────────────────────────────────────────────────────────────┐
│  [Drone]  My Morning Pipeline          [+ Add Drone] [Run All]│
├───────────┬─────────────────────────────────────────────────┤
│  Toolbar  │  Canvas (node graph)                             │
│           │                                                  │
│ [Arrange] │  ┌──────────────┐    ┌──────────────┐           │
│ [Fit]     │  │ MarketScan   │───▶│ ReportWriter │           │
│ [Zoom+/-] │  │ ⏰ 7:00am    │    │ ↳ on success │           │
│ [Minimap] │  │ ● running    │    │ ○ idle       │           │
│           │  └──────────────┘    └──────────────┘           │
│  Filters  │          │                                       │
│ ○ All     │          ▼                                       │
│ ● Running │  ┌──────────────┐                               │
│ ○ Failed  │  │ AlertDrone   │                               │
│ ○ Idle    │  │ ⚡ on failure │                               │
│           │  │ ✗ 3 retries  │                               │
│           │  └──────────────┘                               │
│           │                                      [Minimap]  │
└───────────┴─────────────────────────────────────────────────┘
```

### Node Anatomy

Each node in the graph represents one Drone. Nodes are compact by default and expandable.

**Collapsed node (default):**
```
┌────────────────────────────────┐
│ ● MarketScan          [▶] [⋯] │  ← status dot, name, quick actions
│ ⏰ 0 7 * * *    Last: 2m ago  │  ← primary trigger, last run time
└────────────────────────────────┘
  ↑ output handle          ↑ input handle (for dependency edges)
```

**Expanded node (click to expand):**
```
┌────────────────────────────────┐
│ ● MarketScan          [▶] [⋯] │
│ ⏰ 0 7 * * *  ⚡ pr.opened    │  ← multiple triggers shown
├────────────────────────────────┤
│ Status: running (attempt 1/3)  │
│ Started: 7:02:14 AM            │
│ Runtime: 2m 18s                │
├────────────────────────────────┤
│ Last 3 runs:  ✓ ✓ ✗           │  ← sparkline of recent runs
│ [View logs] [Edit] [Disable]   │
└────────────────────────────────┘
```

### Status Colors

| State | Color | Icon | Description |
|-------|-------|------|-------------|
| `idle` | Gray | ○ | Waiting for next trigger |
| `queued` | Blue | ◔ | Triggered, waiting to start |
| `running` | Green (pulsing) | ● | Agent is active |
| `success` | Green (steady) | ✓ | Last run succeeded |
| `failed` | Red | ✗ | Last run failed, no more retries |
| `retrying` | Yellow | ↺ | Failed, will retry |
| `disabled` | Dimmed | ⊘ | Manually paused |
| `timed_out` | Orange | ⌛ | Run exceeded timeout |

### Edge Types

| Edge | Visual | Meaning |
|------|--------|---------|
| Dependency (success) | Solid green arrow | "Fire when source succeeds" |
| Dependency (failure) | Solid red arrow | "Fire when source fails" |
| Dependency (any) | Solid gray arrow | "Fire when source completes (either)" |
| Data flow | Dashed blue arrow | "Pass output of source as input to target" |

---

## Interaction Design

### Canvas Navigation (standard)
- **Scroll**: zoom in/out
- **Drag canvas**: pan
- **Drag node**: reposition
- **Double-click node**: open edit panel
- **Right-click node**: context menu (Run now, Disable, Edit, Delete, View logs)
- **Right-click canvas**: Add drone, Auto-arrange, Fit to view
- **Drag output handle → input handle**: create dependency edge
- **Click edge**: select (shows delete button)
- **`F` key**: fit all nodes to view
- **`/` key**: open drone search/add palette

### Drone Edit Panel (right-side drawer)
Opens on double-click or "Edit" from context menu. Sections:

1. **Identity** — name, description, icon
2. **Agent** — choose Forge agent or inline (provider + prompt)
3. **Task** — the instruction sent to the agent on each run (supports `{{date}}`, `{{trigger}}` templates)
4. **Triggers** — add/remove triggers; cron shown with human-readable preview ("Every day at 7:00 AM")
5. **Run Policy** — retry count (0-10), retry delay (seconds), timeout (minutes), max concurrency
6. **Environment** — override env vars per drone (layered on top of Forge agent env)
7. **Notifications** — on success / on failure: badge only, sound, OS notification, jekt to agent

### Run Log Panel (bottom drawer, expands on click)
Shows the selected drone's run history as a scrollable timeline:

```
Run #47 — ✓ success — Today 7:02 AM — 3m 12s — triggered by cron
Run #46 — ✗ failed  — Yesterday 7:01 AM — 1m 04s — timed out
  Attempt 1: failed (1m 04s) → Attempt 2: success (2m 08s) → skipped
Run #45 — ✓ success — 2 days ago 7:00 AM — 2m 55s
```

Click any run to expand: shows agent output (first N lines + link to full log), trigger metadata.

### Minimap
Always-visible in bottom-right corner of canvas. Clickable to jump to that region.
Shows nodes as colored dots matching their status color. Especially useful for large drone
fleets (20+ drones).

---

## Data Model

### DroneDefinition (stored in Waveform object store)

```typescript
interface DroneDefinition {
    id: string;                    // uuid
    name: string;
    description?: string;
    icon?: string;                 // FontAwesome name
    enabled: boolean;

    // Agent source — one of:
    forgeAgentId?: string;         // reference to existing Forge agent
    inlineAgent?: {                // or inline definition
        provider: string;          // "claude" | "codex" | "gemini"
        soul?: string;             // CLAUDE.md content
    };

    task: string;                  // prompt template for each run
    // Template vars: {{date}}, {{time}}, {{trigger_type}}, {{trigger_source}},
    //                {{prev_output}}, {{run_number}}

    triggers: DroneTrigger[];

    runPolicy: {
        retryCount: number;        // 0 = no retry, max 10
        retryDelaySecs: number;    // delay between retries
        timeoutMins: number;       // 0 = no timeout
        maxConcurrent: number;     // 1 = no parallel runs of same drone
    };

    notifications: {
        onSuccess: "none" | "badge" | "sound" | "os" | "jekt";
        onFailure: "none" | "badge" | "sound" | "os" | "jekt";
        jektTarget?: string;       // agent name to jekt on completion
    };

    // Canvas position (stored per layout)
    canvasX: number;
    canvasY: number;
}

type DroneTrigger =
    | { type: "cron"; expr: string; timezone?: string }
    | { type: "event"; eventName: string; filter?: string }  // JSONPath filter
    | { type: "dependency"; droneId: string; on: "success" | "failure" | "any" }
    | { type: "webhook"; path: string; secret?: string }
    | { type: "manual" }
    | { type: "watchdog"; droneId: string; idleMins: number };
```

### DroneRun (stored per execution)

```typescript
interface DroneRun {
    id: string;
    droneId: string;
    runNumber: number;
    triggerType: DroneTrigger["type"];
    triggerSource?: string;        // event name, cron expr, parent drone id, etc.
    attempt: number;               // 1-based, increments on retry
    maxAttempts: number;
    state: "queued" | "running" | "success" | "failed" | "cancelled" | "timed_out";
    startedAt: number;             // unix ms
    endedAt?: number;
    outputSummary?: string;        // first 500 chars of agent output
    outputBlockId?: string;        // blockId holding full output (scrollback)
    errorMsg?: string;
}
```

---

## Backend Architecture

### Drone Scheduler Service (Rust, `agentmuxsrv-rs`)

A new `DroneScheduler` module runs inside the existing sidecar process. It:

1. **Loads drone definitions** from the Waveform object store on startup and watches
   for changes (reactive via the existing WOS subscription pattern).

2. **Cron loop**: Evaluates all `cron` triggers every minute using a `cron` crate
   (`tokio-cron-scheduler` or `cron` + `tokio::time::interval`). Fires a run when
   the cron expression matches the current UTC minute.

3. **Event bus**: Subscribes to the internal AgentBus. When an event arrives matching
   any drone's `event` trigger, fires that drone. Also listens for `DroneRunCompleted`
   events to evaluate `dependency` triggers.

4. **Webhook listener**: Optionally opens a local HTTP server (or registers routes on
   the existing axum server) per drone with a `webhook` trigger. Validates HMAC-SHA256
   signature if `secret` is set.

5. **Run queue**: A Tokio MPSC channel per drone (bounded by `maxConcurrent`). Runs
   are dequeued and executed by spawning a subprocess (same `SubprocessController`
   pattern used by Agent panes). Output is streamed to a run-specific block.

6. **Retry loop**: On failed runs, schedules a retry after `retryDelaySecs` up to
   `retryCount` attempts. Each retry increments `attempt` counter and emits a
   `DroneRunRetrying` event.

7. **Watchdog**: A secondary interval loop checks idle durations and fires watchdog
   drones if a watched drone hasn't run in `idleMins`.

### IPC Commands (new, via existing RpcApi pattern)

```
ListDronesCommand      → DroneDefinition[]
GetDroneCommand        → DroneDefinition
CreateDroneCommand     → DroneDefinition
UpdateDroneCommand     → DroneDefinition
DeleteDroneCommand     → void
EnableDroneCommand     → void
DisableDroneCommand    → void
TriggerDroneCommand    → DroneRun           (manual trigger)
CancelRunCommand       → void
ListRunsCommand        → DroneRun[]         (paginated, newest first)
GetRunOutputCommand    → string             (full agent output for a run)
```

### Event Bus Integration

Drone publishes and consumes these events on the internal bus:

| Event | Direction | Payload |
|-------|-----------|---------|
| `drone.triggered` | Publish | droneId, triggerType, runId |
| `drone.run.started` | Publish | droneId, runId, attempt |
| `drone.run.completed` | Publish | droneId, runId, state, output |
| `drone.run.retrying` | Publish | droneId, runId, attempt, nextAttemptIn |
| `agent.*` | Subscribe | Any event from Agent panes (for event triggers) |
| External events | Subscribe | AgentBus events forwarded to internal bus |

---

## Graph Library

### Recommendation: `solid-flow` + `@antv/g6`

AgentMux uses SolidJS. Options evaluated:

| Library | Status | Notes |
|---------|--------|-------|
| `react-flow` (`@xyflow/react`) | ❌ React-only | Not compatible |
| `solid-flow` | ✅ SolidJS native | Direct SolidJS components, small, maintained |
| `solid-g6` (SolidJS wrapper for `@antv/g6`) | ✅ SolidJS native | Alibaba-backed, more powerful layouts |
| `cytoscape.js` | ✅ Framework-agnostic | Excellent for large graphs, less polished UX |
| `JointJS` / `jsPlumb` | ✅ Framework-agnostic | Commercial, mature, more code to wire up |

**Recommended approach:** Start with `solid-flow` for the interactive canvas.
It provides pan/zoom viewport, node drag, edge drawing, and minimap out of the box
with SolidJS signal-native reactivity. If we need >100 nodes or advanced layout
algorithms (force-directed, hierarchical), migrate canvas rendering to `@antv/g6`
which handles large graphs with WebGL.

Custom node and edge components are pure SolidJS — fully composable with our existing
design system (same color vars, same fonts, same icon library).

### Node Component Structure

```tsx
// DroneNode.tsx — SolidJS component
function DroneNode(props: { drone: DroneDefinition; latestRun?: DroneRun }) {
    const [expanded, setExpanded] = createSignal(false);
    const status = () => latestRunState(props.latestRun);

    return (
        <div class={`drone-node drone-node--${status()}`}>
            <div class="drone-node__header">
                <StatusDot state={status()} />
                <span class="drone-node__name">{props.drone.name}</span>
                <RunNowButton droneId={props.drone.id} />
                <NodeMenu droneId={props.drone.id} />
            </div>
            <div class="drone-node__triggers">
                <TriggerBadge triggers={props.drone.triggers} />
                <LastRunBadge run={props.latestRun} />
            </div>
            <Show when={expanded()}>
                <DroneNodeDetails drone={props.drone} latestRun={props.latestRun} />
            </Show>
            {/* Connection handles — output (bottom) and input (top) */}
            <Handle type="source" position="bottom" />
            <Handle type="target" position="top" />
        </div>
    );
}
```

---

## ViewModel Integration

Following the existing `ViewModel` pattern (see `AgentViewModel`, `SysinfoViewModel`):

```typescript
class DroneViewModel implements ViewModel {
    viewType = "drone";
    viewIcon = () => "drone";          // or "robot", "microchip"
    viewName = () => "Drone";
    viewText = () => this.summaryText();
    noPadding = () => true;

    dronesAtom: SignalAtom<DroneDefinition[]>;
    selectedDroneId: SignalAtom<string | null>;
    latestRunsAtom: SignalAtom<Map<string, DroneRun>>;    // droneId → latest run
    canvasStateAtom: SignalAtom<CanvasState>;              // viewport transform
    editPanelOpen: SignalAtom<boolean>;

    summaryText() {
        const drones = this.dronesAtom();
        const running = drones.filter(d => /* latest run is running */).length;
        const failed = drones.filter(d => /* latest run is failed */).length;
        if (running > 0) return `${running} running`;
        if (failed > 0) return `${failed} failed`;
        return `${drones.length} drones`;
    }
}
```

---

## Best Practices Applied

### From Workflow Orchestration Research

**1. Unified trigger model (Windmill pattern)**
A single drone accepts any combination of triggers — cron + event + dependency. No
separate "scheduled job" vs "event listener" concepts. Reduces cognitive load.

**2. Four-layer status visibility (from AI Design Patterns)**
- **Ambient**: Status dot in pane header shows fleet health at a glance (all green / any red)
- **Progress**: Expanded node shows runtime, attempt counter, percentage if available
- **Attention**: OS notification or jekt to agent only when attention is needed (failure, human gate)
- **Summary**: Run history panel shows outcome counts per drone

**3. Blast radius visualization (Dagster/Prefect pattern)**
When a drone fails, downstream dependency edges and nodes are dimmed/highlighted to show
what won't run as a result. Prevents confusion about cascading failures.

**4. Retry with attempt indicator (Temporal pattern)**
Retries are first-class. The node shows "↺ attempt 2/3" with a yellow status dot during
retries, not a generic "running" state. Retry history is per-run, not per-drone.

**5. Cron expression with human-readable preview**
Cron fields show a plain-English translation inline:
```
0 7 * * 1-5   →   "Weekdays at 7:00 AM"
*/15 * * * *  →   "Every 15 minutes"
0 0 1 * *     →   "Monthly on the 1st at midnight"
```
Uses `cronstrue` or equivalent library.

**6. Time-travel run inspection (LangGraph pattern)**
Every run has its full output stored in a block. The run history panel links to it.
Future: allow replaying a run's input with edited task prompt ("edit and re-run").

**7. Dependency visualization over list view (LangGraph Studio)**
Jobs/drones are not a flat list. They are a graph. The graph is the primary view
because dependency relationships are the most important thing to understand when
debugging failures and understanding execution order.

**8. Selective retry (Airflow pattern)**
When a run fails with partial output (e.g., processed 80 of 100 items), a "Retry
failed items only" action is available if the agent supports it via task templates:
`{{failed_items}}` variable populated from previous run output.

---

## Implementation Phases

### Phase 1 — Canvas + Manual Drones (MVP)
- `DroneViewModel` and `DroneView` components (SolidJS)
- `solid-flow` canvas with custom DroneNode
- Create/edit/delete drones via edit panel
- `manual` trigger only — run on demand
- Run history per drone (in-memory, no persistence)
- Status dot + pane header count
- Register `viewType = "drone"` in blocktypes

### Phase 2 — Cron + Event Triggers
- `DroneScheduler` in Rust sidecar
- `cron` trigger with `tokio-cron-scheduler`
- `event` trigger via internal AgentBus subscription
- Run record persistence (Waveform object store or SQLite)
- Retry policy enforcement
- Cron human-readable preview in edit panel

### Phase 3 — Dependency Graph + Notifications
- `dependency` trigger (success / failure / any)
- Edge drawing UI (drag from output handle to input handle)
- Blast radius highlighting on failure
- Downstream node dimming
- Jekt notifications on completion
- OS notifications via existing notification plugin

### Phase 4 — Webhooks + Advanced
- `webhook` trigger with local HTTP listener
- `watchdog` trigger
- Data flow edges (`{{prev_output}}` piped to downstream task)
- Minimap
- Auto-arrange (hierarchical layout)
- Export/import drone definitions as JSON
- CEF: DevTools accessible on :9222 for debugging drone output

---

## Open Questions

1. **Persistence backend**: Waveform object store (current pattern) or a dedicated
   SQLite file for run history? Runs accumulate quickly; WOS may not be designed
   for append-only time-series. Recommendation: WOS for definitions, SQLite for runs.

2. **Output storage**: Full agent output per run could be large. Options: keep last N
   runs of output only, compress old runs, or store only the first/last 1000 lines.

3. **Concurrency with Agent panes**: If a Drone fires while the user has an interactive
   Agent pane open using the same Forge agent, do they share a process? Recommendation:
   Drones always get a dedicated process; no sharing with interactive panes.

4. **AgentBus event schema**: What events does AgentBus publish that drones should
   react to? Need a canonical event registry (e.g., `pr.opened`, `deploy.completed`,
   `agent.finished`).

5. **Timezone handling for cron**: Default to local system timezone. Allow per-drone
   timezone override. Display next-run time in local time.

6. **Security for webhooks**: Local-only webhooks (bind to 127.0.0.1) by default.
   Optional HMAC secret. No remote exposure without explicit user action.

---

## Files to Create

```
frontend/app/view/drone/
  index.ts                    — exports DroneViewModel, makeViewModelTypeForDrone
  drone-model.ts              — DroneViewModel class
  drone-view.tsx              — main canvas component
  drone-node.tsx              — individual node component
  drone-edit-panel.tsx        — right-side edit drawer
  drone-run-log.tsx           — run history bottom drawer
  drone-types.ts              — DroneDefinition, DroneRun, DroneTrigger types
  drone-store.ts              — SolidJS store/atoms for drones + runs
  drone-utils.ts              — cron preview, status helpers
  drone.css                   — canvas + node styles

agentmuxsrv-rs/src/
  drone/
    mod.rs                    — DroneScheduler struct
    scheduler.rs              — cron loop + event subscription
    runner.rs                 — spawns SubprocessController per run
    webhook.rs                — local HTTP webhook listener
    store.rs                  — drone definition + run persistence
    events.rs                 — internal event types
```
