# Spec: Per-Pane CPU + Memory Metrics Badge

**Goal:** Show real-time CPU % and memory (RSS) for the process running inside each pane, displayed as a compact badge in the pane header.

**Issue:** #88

**Status:** Ready for implementation.

---

## Design

```
┌─ bash  ─────────── 2.1%  48M ───────────── □ ✕ ┐
│                                                   │
│  $ cargo build --release                          │
│  Compiling agentmux v0.32.2                       │
└───────────────────────────────────────────────────┘
```

A compact badge in the header shows CPU % and memory RSS for the pane's process. Updates every tick (same interval as global sysinfo, default 1s).

### Which Panes Show Metrics

| View | Shows? | Why |
|------|--------|-----|
| `term` | Yes | Shell process |
| `agent` | Yes | Agent CLI process |
| `sysinfo` | No | Already shows global metrics |
| `launcher` / `help` | No | No controller process |

Rule: show badge when `blockcontroller.status == "running"`.

---

## Backend Changes

### 1. Block PID Registry

**File:** `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs`

After `pair.slave.spawn_command(cmd)` succeeds (line ~514), capture the PID:

```rust
let pid = child.process_id(); // portable_pty Child::process_id() -> Option<u32>
```

Store in a global registry so sysinfo can look up PIDs by blockId:

**New file:** `agentmuxsrv-rs/src/backend/blockcontroller/pidregistry.rs`

```rust
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

static BLOCK_PIDS: LazyLock<RwLock<HashMap<String, u32>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn register(block_id: &str, pid: u32) {
    BLOCK_PIDS.write().unwrap().insert(block_id.to_string(), pid);
}

pub fn unregister(block_id: &str) {
    BLOCK_PIDS.write().unwrap().remove(block_id);
}

pub fn get_all() -> Vec<(String, u32)> {
    BLOCK_PIDS.read().unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}
```

**Registration points in shell.rs:**
- Register after spawn: `pidregistry::register(&self.block_id, pid)` (after line 523)
- Unregister on exit: `pidregistry::unregister(&block_id)` (in the wait task, after line 715)
- Unregister on kill: `pidregistry::unregister(&block_id)` (in drop/cleanup paths)

### 2. New Event Type

**File:** `agentmuxsrv-rs/src/backend/wps.rs`

```rust
pub const EVENT_BLOCK_STATS: &str = "blockstats";
```

### 3. Per-Process Collection in Sysinfo Loop

**File:** `agentmuxsrv-rs/src/backend/sysinfo.rs`

After publishing global metrics, iterate block PIDs and publish per-block events:

```rust
use sysinfo::Pid;
use crate::backend::blockcontroller::pidregistry;
use crate::backend::wps::EVENT_BLOCK_STATS;

// Inside run_sysinfo_loop, after broker.publish(event):

let block_pids = pidregistry::get_all();
for (block_id, pid) in &block_pids {
    // Targeted refresh — only this process, cheap
    sys.refresh_process(Pid::from_u32(*pid));
    if let Some(process) = sys.process(Pid::from_u32(*pid)) {
        let mut block_values = HashMap::new();
        block_values.insert("cpu".to_string(), process.cpu_usage() as f64);
        block_values.insert("mem".to_string(), process.memory() as f64); // bytes
        block_values.insert("pid".to_string(), *pid as f64);

        let block_ts = TimeSeriesData { ts: now, values: block_values };
        let block_event = WaveEvent {
            event: EVENT_BLOCK_STATS.to_string(),
            scopes: vec![format!("block:{}", block_id)],
            sender: String::new(),
            persist: 0, // no history needed
            data: serde_json::to_value(&block_ts).ok(),
        };
        broker.publish(block_event);
    }
}
```

**Performance:** `sys.refresh_process(pid)` queries one process — ~0.1ms per process. With 10 panes open, adds ~1ms per tick. Negligible.

### 4. sysinfo Crate: Refresh Semantics

The `sysinfo::System` needs `refresh_process_specifics` or `refresh_process` to query individual processes. Current code uses `sys.refresh_cpu_usage()` (global only). The per-process call is additive — both can coexist.

**Important:** `process.cpu_usage()` requires TWO refreshes to compute delta. On first refresh, CPU will be 0%. This is fine — the badge will show 0% briefly on first tick, then real values.

---

## Frontend Changes

### 5. Event Type Constant

**File:** `frontend/types/custom.d.ts` (or wherever event types are declared)

Ensure `"blockstats"` is a recognized event type for `waveEventSubscribe`.

### 6. `useBlockStats` Hook

**New file:** `frontend/app/hook/useBlockStats.ts`

```typescript
import { createSignal, createEffect, onCleanup } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";

export interface BlockStats {
    cpu: number;  // percentage (0-100+)
    mem: number;  // bytes
}

export function useBlockStats(blockId: string): () => BlockStats | null {
    const [stats, setStats] = createSignal<BlockStats | null>(null);

    createEffect(() => {
        const unsub = waveEventSubscribe({
            eventType: "blockstats",
            scope: `block:${blockId}`,
            handler: (event) => {
                const data = event.data;
                if (data?.values) {
                    setStats({
                        cpu: data.values.cpu ?? 0,
                        mem: data.values.mem ?? 0,
                    });
                }
            },
        });
        onCleanup(() => unsub());
    });

    return stats;
}
```

### 7. `BlockStatsBadge` Component

**New file:** `frontend/app/element/blockstats.tsx`

```tsx
import { Show, createMemo } from "solid-js";
import { useBlockStats } from "@/app/hook/useBlockStats";
import "./blockstats.scss";

function formatMem(bytes: number): string {
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}K`;
    if (bytes < 1024 * 1024 * 1024) return `${Math.round(bytes / (1024 * 1024))}M`;
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)}G`;
}

export function BlockStatsBadge(props: { blockId: string }) {
    const stats = useBlockStats(props.blockId);

    const cpuClass = createMemo(() => {
        const s = stats();
        if (!s) return "";
        if (s.cpu > 90) return "cpu-high";
        if (s.cpu > 50) return "cpu-medium";
        return "";
    });

    return (
        <Show when={stats()}>
            {(s) => (
                <div class={`block-stats-badge ${cpuClass()}`}>
                    <span class="stats-cpu">{s().cpu.toFixed(1)}%</span>
                    <span class="stats-mem">{formatMem(s().mem)}</span>
                </div>
            )}
        </Show>
    );
}
```

### 8. Styling

**New file:** `frontend/app/element/blockstats.scss`

```scss
.block-stats-badge {
    display: flex;
    gap: 6px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    opacity: 0.6;
    white-space: nowrap;
    padding: 0 4px;

    .stats-cpu, .stats-mem {
        color: var(--secondary-text-color);
    }

    &.cpu-medium .stats-cpu {
        color: var(--warning-color, #f59e0b);
    }

    &.cpu-high .stats-cpu {
        color: var(--error-color, #ef4444);
    }
}
```

### 9. Header Integration

**File:** `frontend/app/block/blockframe.tsx`

In the header, render the badge between the text elems and end icons:

```tsx
import { BlockStatsBadge } from "@/element/blockstats";

// In BlockFrame_Default_Header, after headerTextElems and before EndIcons:
<div class="block-frame-stats-wrapper">
    <BlockStatsBadge blockId={props.nodeModel.blockId} />
</div>
```

The badge auto-hides (via `<Show when={stats()}>`) when no `blockstats` events are received — covers views without controllers.

---

## Implementation Order

| Step | Layer | File(s) | Description |
|------|-------|---------|-------------|
| 1 | Backend | `pidregistry.rs` (new) | Block PID registry (register/unregister/get_all) |
| 2 | Backend | `shell.rs` | Register PID after spawn, unregister on exit |
| 3 | Backend | `wps.rs` | Add `EVENT_BLOCK_STATS` constant |
| 4 | Backend | `sysinfo.rs` | Per-process metrics collection + publish |
| 5 | Frontend | `useBlockStats.ts` (new) | Event subscription hook |
| 6 | Frontend | `blockstats.tsx` (new) | Badge component + SCSS |
| 7 | Frontend | `blockframe.tsx` | Render badge in header |

---

## Edge Cases

1. **Process exits** → PID unregistered, no more events, badge disappears (via `<Show when={stats()}>`)
2. **Process not yet spawned** → no PID registered, no events, badge hidden
3. **Multiple refreshes needed for CPU** → first tick shows 0%, subsequent ticks show real values
4. **Child processes** → Phase 1 tracks only the direct PTY process, not children. A `cargo build` spawning many rustc processes won't be summed. This is intentional — Phase 2 can add process tree walking.
5. **High pane count** → 20 panes = 20 process refreshes per tick ≈ 2ms. Fine.

---

## Out of Scope (Phase 2)

- Process tree / child process summation
- GPU, disk, network per pane
- Historical sparkline per pane
- Per-pane resource limits / kill actions
- Click badge to show detailed metrics popup
