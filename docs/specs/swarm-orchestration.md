<!-- Captured from GitHub issue #101 — agentmuxai/agentmux -->

## Overview

Coordinate N parallel AI agents on a shared codebase: each agent works on isolated tasks from a shared queue, a lifecycle manager auto-routes CI failures and review comments, and a reviewer agent gates merges.

**Spec:** `specs/swarm-orchestration.md`

Synthesizes ideas from:
- **[oompa](https://github.com/nbardy/oompa)** — filesystem task queue, planner/executor/reviewer model, `oompa.json` worker composition, `__DONE__` termination
- **[agent-orchestrator](https://github.com/ComposioHQ/agent-orchestrator)** — lifecycle state machine, JSONL activity detection, auto-reactions to CI/review
- **AgentMux jekt** — push-based task delivery to complement the pull-based queue

---

## 1. Filesystem Task Queue

A simple, git-friendly task protocol. No database, no broker — pure filesystem.

```
{workspace}/tasks/
├── pending/   ← orchestrator writes tasks here
├── current/   ← agent moves file here to claim (atomic rename)
└── complete/  ← agent moves here when done
```

**Task file:**
```json
{
  "id": "001",
  "summary": "Add JWT authentication",
  "description": "Implement JWT auth for /api/users...",
  "acceptance": ["POST /auth/login returns JWT", "Tests pass"],
  "assigned_to": null
}
```

Agents claim tasks via atomic `rename()` — no race conditions, works in containers. Tasks persist through crashes. Orchestrator can write tasks AND jekt "new task available" to skip polling delay.

---

## 2. Worker Composition (`swarm.json`)

Declarative swarm config — Forge's "Launch Swarm" reads this:

```json
{
  "swarm_id": "feature-auth",
  "workers": [
    { "role": "planner",  "agent": "AgentX",  "count": 1, "can_create_tasks": true },
    { "role": "executor", "provider": "claude","count": 3, "can_create_tasks": false },
    { "role": "reviewer", "provider": "claude","count": 1, "triggers_on": ["pr_opened"] }
  ],
  "termination": { "signal": "__DONE__", "conditions": ["all_tasks_complete"] }
}
```

| Role | Purpose |
|------|---------|
| `planner` | Breaks feature into tasks, monitors progress |
| `executor` | Claims + completes tasks, creates PRs |
| `reviewer` | Validates PRs before merge |

---

## 3. Lifecycle Manager (`claw watch`)

A polling daemon (30s interval) that watches agent PRs and auto-routes events:

**State machine:**
```
idle → spawning → working → pr_open → review_pending → approved → mergeable → merged → done
                           ↘ ci_failed   ↘ changes_requested
                           ↘ needs_input ↘ stuck (notify human)
```

**Auto-reactions:**
- CI failed → read failure logs → jekt to the executor that created the PR
- Review comments → jekt review feedback to executor
- PR approved + green → notify human, add to merge queue
- Agent stuck > threshold → escalate to human via Slack/desktop

**State persistence:** Flat files at `~/.claw/watch-state/{swarm_id}/`

---

## 4. Activity Detection via JSONL

Claude Code writes session state to `~/.claude/projects/{path}/*.jsonl`. Read from containers via `docker exec` or volume mount.

| Last entry type | Agent state |
|----------------|-------------|
| `tool_use`, `progress` | `active` |
| `assistant`, `result` | `ready` |
| `permission_request` | `waiting_input` |
| `error` | `blocked` |
| Timestamp > 10min | `idle` |

Surfaces in `claw status`. Alerts on `blocked` / `stuck`.

---

## 5. Reviewer Gate

When an executor creates a PR:
1. Lifecycle manager detects `pr_open` state
2. Reads PR diff via GitHub API
3. Writes `tasks/pending/review-{pr_number}.json`
4. Jekts reviewer "new review task"
5. Reviewer claims, posts GitHub review
6. If approved → mark mergeable
7. If changes_requested → jekt executor with review comments, create new fix task

---

## 6. Termination

Swarm terminates when:
- **`__DONE__`** token emitted by planner
- `tasks/pending/` + `tasks/current/` both empty
- Max cycles exceeded
- Manual: `claw stop --swarm feature-auth`

---

## 7. Prompt Composition

Workers receive layered prompts with file-include directives:
```
#include "config/prompts/security-rules.md"
#include "config/prompts/coding-standards.md"
```

Includes resolved relative to workspace root. Shared instruction files reused across workers.

---

## 8. Forge Integration

New **Swarm** tab in the Forge:

```
┌────────────────────────────────────────────────┐
│  THE FORGE     [Agents] [Swarms] [Skills]       │
├─────────────────────────────────────────────────┤
│  feature-auth                                   │
│  3 executors · 1 reviewer                       │
│  Tasks: 2 complete / 1 current / 4 pending      │
│  ● running   [View Live] [Stop]                 │
└─────────────────────────────────────────────────┘
```

"Launch Swarm" button reads `swarm.json`, spawns workers per composition.

---

## 9. Notification System

| Event | Channel |
|-------|---------|
| Agent stuck | Slack + Desktop |
| PR ready to merge | Slack + Desktop |
| CI repeatedly failing | Slack |
| Swarm complete | Desktop |

---

## Implementation Phases

1. **Task queue protocol** — `claw task` commands, filesystem layout doc
2. **`claw watch` daemon** — GitHub polling, CI routing, state persistence
3. **Activity detection** — JSONL reader, state classification, `claw status`
4. **Swarm config + Forge Swarm tab** — `swarm.json`, Launch Swarm, status view
5. **Reviewer gate** — Review task routing, changes-requested feedback loop
6. **Notifications** — Slack + desktop

---

## Acceptance Criteria

- [ ] `claw task create "Add JWT auth"` writes to `tasks/pending/`
- [ ] Agent claims task via atomic rename, completes it, moves to `tasks/complete/`
- [ ] `claw watch` detects CI failure, jekts failure logs to the responsible agent
- [ ] Reviewer agent auto-claims review tasks, posts GitHub review
- [ ] `claw status` shows per-agent state: `active / ready / waiting_input / blocked / idle`
- [ ] Swarm terminates cleanly when `__DONE__` emitted or queue drained
- [ ] Forge Swarm tab shows live task counts and agent statuses
- [ ] Prompt includes work: `#include "config/prompts/security-rules.md"` is resolved at launch
