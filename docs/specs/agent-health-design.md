# Agent Health/Liveness Detection — Design Spec

> **Date:** 2026-03-17
> **Problem:** A CLI process can be "running" but the agent is broken (400 errors, rate limits, auth expired, etc.). AgentMux has no way to detect or surface this.

---

## Design Principles

1. **Primary signal is output activity**, not process vitals
2. **Never auto-kill running processes** — always ask the user
3. **Default unknown errors to Fatal** — safer to over-alert
4. **Health monitoring in Rust backend**, UI rendering in React
5. **Notifications are tiered**: badge for all transitions, toast only for fatal
6. **Orthogonal to `shellprocstatus`** — process can be "running" but agent "dead"

---

## 1. Health State Machine

```
AgentHealth:
  Healthy     — Normal operation, receiving output
  Idle        — Between turns, waiting for input
  Degraded    — Working but impaired (rate limits, slow)
  Stalled     — No output for >30s during active turn
  Dead        — Requires intervention (fatal error or >120s silence)
  Exited(i32) — Process terminated with exit code
```

Composite check:
```
if !process.is_alive() → Exited(exit_code)
if errors.has_fatal()  → Dead
if silence > 120s      → Dead
if silence > 30s       → Stalled
if errors.is_degraded()→ Degraded
if !active_turn        → Idle
else                   → Healthy
```

---

## 2. Health Signals from CLI Output

### Output Activity Watchdog (Primary)

```
last_output_ts: Instant
last_meaningful_ts: Instant   (excludes rate_limit_events)
active_turn: bool

check() [called every 5s]:
  if !active_turn: return Idle
  silence = now() - last_meaningful_ts
  if silence > 120s: return Dead
  if silence > 30s:  return Stalled
  return Healthy
```

Timeouts are generous because Claude Code can legitimately pause during tool use.

### Error Rate Sliding Window

```rust
struct ErrorTracker {
    window: VecDeque<(Instant, ErrorClass)>,
    window_duration: Duration,  // 5 minutes
}

// Thresholds:
// 1 fatal error in any window       → Dead
// 5+ transient errors in 5 minutes  → Degraded
// 3+ consecutive transient w/o success → Stalled
```

### Process Vitals (Supplementary)

Weak signal, but catches zombie processes:
- Check process alive via child handle
- Fallback, not primary

---

## 3. Error Classification

| Signal | Class | Retry? |
|--------|-------|--------|
| `rate_limit_event` | Transient | Yes (CLI handles) |
| HTTP 500/502/503 | Transient | Yes |
| Connection refused/timeout | Transient | Yes |
| HTTP 401 / auth expired | Fatal | No |
| HTTP 403 / account suspended | Fatal | No |
| Model not found / 404 | Fatal | No |
| Malformed request / 400 | Fatal | No |
| Process exit != 0 | Depends | Check stderr |
| Silence > threshold | Stalled | Probe first |
| Unknown errors | Fatal | No (safer) |

### Parsing Strategy

```typescript
function classifyEvent(event: StreamEvent): ErrorClass | null {
    if (event.type === 'rate_limit_event') return Transient;
    if (event.type === 'result' && event.is_error) {
        const msg = (event.error_message || '').toLowerCase();
        if (/unauthorized|401|forbidden|403|token expired/.test(msg)) return Fatal;
        if (/overloaded|503|500|rate/.test(msg)) return Transient;
        return Fatal; // Unknown errors default to fatal
    }
    return null;
}
```

---

## 4. Recovery Strategies

### Transient Errors
- CLI handles its own retries — parent monitors whether retries succeed
- If rate_limit_events dominate for >2min with no progress → transition to Stalled
- Show user: "Wait" or "Restart Agent"
- **Do NOT auto-restart** for transient errors

### Fatal Errors
```
┌──────────────────────────────────────────┐
│  ⚠ Agent "Claude-1" needs attention      │
│  Authentication expired.                 │
│  [Re-authenticate]  [Restart]  [Dismiss] │
└──────────────────────────────────────────┘
```

Per fatal type:
- Auth expired → Run `claude auth login`, then restart
- Account suspended → Link to account page
- Model deprecated → Offer model switch
- Unknown → Show raw error, offer restart

### Stalled/Dead
- **NEVER auto-kill.** Always ask user.
- Show: "Agent appears unresponsive. [Kill & Restart] [Keep Waiting]"
- Cost of killing working-but-slow > cost of waiting for truly dead

---

## 5. UI Indicators

### Per-Pane Health Badge

In the pane header, next to the agent name:

| Color | State | Meaning |
|-------|-------|---------|
| Green | Healthy | Actively producing output |
| Gray | Idle | Between turns |
| Yellow | Degraded | Rate limited / slow |
| Orange (pulse) | Stalled | No output >30s during active turn |
| Red | Dead | Fatal error or >120s silence |
| None | Exited | Process terminated (exit code in tooltip) |

```css
.health-badge {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    transition: background-color 0.3s ease;
}
.health-badge.pulse {
    animation: pulse 2s ease-in-out infinite;
}
```

### Notification Rules
- **Toast only for Fatal** — requires user action
- **Badge color only for Degraded/Stalled** — visible, not interruptive
- **Aggregate**: "2 agents need attention" not one per agent
- **Sound**: optional, off by default

### Status Bar Summary (if applicable)
```
Agents: 3 ● healthy  1 ● stalled  1 ● error
```

---

## 6. Architecture

### Where Each Component Lives

**Rust backend (agentmuxsrv-rs):**
- `HealthMonitor` struct per block — owns watchdog timer, error tracker, process check
- Emits `health_changed` events via Tauri event system on state transitions

**TypeScript frontend:**
- Subscribes to `health_changed` events per block (same pattern as `controllerstatus`)
- Jotai atom: `agentHealthAtom(blockId)` → `AgentHealth`
- Renders badge, notifications, "Fix" action buttons

**Event flow:**
```
PTY output → stdout_reader (Rust)
  → parse stream-json events
  → feed to HealthMonitor
  → detect transition
  → emit Tauri event: { blockId, health: "stalled", detail: "no output 35s" }
  → Frontend atom updates
  → Badge color changes
  → If fatal: toast notification
```

### Integration with Existing System

Keep health orthogonal to `BlockControllerRuntimeStatus`:
- `shellprocstatus` = is the process running?
- `agentHealth` = is the agent functioning correctly?

```typescript
const displayStatus = useMemo(() => {
    if (procStatus === 'done') return 'exited';
    if (procStatus === 'init') return 'starting';
    return agentHealth; // procStatus === 'running'
}, [procStatus, agentHealth]);
```

### Data Model

Block metadata (persisted):
```json
{
    "agent:health": "healthy",
    "agent:last_output_ts": 1742000000,
    "agent:error_count": 0,
    "agent:last_error": null
}
```

Sliding window error tracker: in-memory only (resets on restart = correct behavior).

---

## 7. Implementation Priority

1. **Process timeout** — `tokio::time::timeout()` in process_waiter, configurable (default 5min)
2. **Error event pattern matching** — classify rate_limit, auth, API errors in translator
3. **Output activity watchdog** — 30s/120s thresholds in Rust, emit health events
4. **Health badge UI** — colored dot in pane header
5. **Stderr forwarding** — surface critical stderr to user as DocumentNodes
6. **Session validation** — check first event is `system/init` after spawn
7. **Fatal error toasts** — aggregate notifications with recovery actions
