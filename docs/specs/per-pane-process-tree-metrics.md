# Spec: Per-Pane Process Tree CPU + Memory Metrics

**Status:** Draft
**Version:** 0.32.74
**Issue:** #88 (partial — extends per-pane metrics to full process tree)

---

## Problem

The `BlockStatsBadge` in each pane header shows CPU% and memory for the **shell process only** (the single PID registered at PTY spawn). Child processes — the actual workloads — are invisible.

Examples of what gets missed:
- `cargo build` spawned from a bash pane (multiple `rustc` workers)
- `claude -p ...` agent CLI (spawned by the agent controller shell)
- `npm run dev` → `node`, `vite`, etc.
- Any tool the user runs interactively

The badge shows 0.0% CPU while the machine is visibly loaded. This is misleading.

---

## Goal

Show the **aggregate CPU% and RSS** for the shell process and all its descendants, so the badge reflects the true load of whatever is running inside the pane.

---

## Current Architecture

```
pidregistry: { blockId → shellPid }
                          ↓
sysinfo loop: refresh_processes_specifics(Some([shellPid, ...]))
                          ↓
publish blockstats event: { cpu: shellCpu, mem: shellMem }
```

Only one PID per block, no tree traversal.

---

## Proposed Architecture

```
pidregistry: { blockId → shellPid }
                          ↓
sysinfo loop:
  1. refresh_processes_specifics(All, minimal kind)  ← build parent map
  2. BFS from each shellPid → descendant PID set
  3. refresh_processes_specifics(Some(all_pids), full kind)  ← CPU + mem
  4. sum per block
                          ↓
publish blockstats event: { cpu: sumCpu, mem: sumMem, pids: count }
```

---

## Backend Implementation

### Step 1 — Cheap full-process parent map

sysinfo 0.34 allows a partial refresh:

```rust
sys.refresh_processes_specifics(
    ProcessesToUpdate::All,
    false,  // remove_dead_processes = false (we handle that ourselves)
    ProcessRefreshKind::new(),  // minimal: gets PID, PPID, name — no CPU/mem yet
);
```

`ProcessRefreshKind::new()` is cheap — no CPU accounting, no memory query. This gives us
`process.parent()` for every process on the system. On a typical desktop this is ~200-400
processes and takes <1ms.

> **Note on `remove_dead_processes`:** pass `false` here to avoid purging stale entries
> before we've had a chance to query them in step 3. The targeted refresh in step 3 will
> simply find no entry for a PID that has exited.

### Step 2 — BFS descendant collection

```rust
fn collect_descendants(sys: &sysinfo::System, root: Pid) -> Vec<Pid> {
    // Build parent→children adjacency from all refreshed processes.
    // This is O(N) where N = total process count (~300 typical).
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, proc) in sys.processes() {
        if let Some(ppid) = proc.parent() {
            children.entry(ppid).or_default().push(*pid);
        }
    }

    // BFS from root
    let mut result = vec![root];
    let mut queue = VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        if let Some(kids) = children.get(&pid) {
            for &child in kids {
                result.push(child);
                queue.push_back(child);
            }
        }
    }
    result
}
```

Cap the result at **64 PIDs per block** as a safety guard against pathological trees
(e.g., a `make -j64`). This prevents a single block from dominating the refresh budget.

### Step 3 — Targeted refresh with full metrics

After collecting all descendant PID sets across all registered blocks, merge into a
single deduplicated slice (a process can only be a child of one block) and do one
targeted refresh:

```rust
let all_pids: Vec<Pid> = all_descendant_sets
    .values()
    .flatten()
    .copied()
    .collect::<HashSet<_>>()
    .into_iter()
    .collect();

sys.refresh_processes_specifics(
    ProcessesToUpdate::Some(&all_pids),
    true,   // remove_dead_processes = true for this pass
    ProcessRefreshKind::everything(),
);
```

### Step 4 — Aggregate per block

```rust
for (block_id, pids) in &all_descendant_sets {
    let mut total_cpu: f64 = 0.0;
    let mut total_mem: u64 = 0;
    let mut live_count: u32 = 0;

    for pid in pids {
        if let Some(proc) = sys.process(*pid) {
            total_cpu += proc.cpu_usage() as f64;
            total_mem += proc.memory();
            live_count += 1;
        }
    }

    // CPU is already per-core (can exceed 100% on multi-core).
    // Clamp display at 999% to avoid badge overflow; raw value stored.
    let block_values = HashMap::from([
        ("cpu".to_string(), total_cpu),
        ("mem".to_string(), total_mem as f64),
        ("pids".to_string(), live_count as f64),
    ]);
    // ... publish blockstats event as before
}
```

**CPU semantics:** sysinfo reports per-process CPU as a percentage of one core
(0–100%). Summing across a tree means values can exceed 100% — this is correct and
matches how `htop` displays process trees. The badge should show raw (e.g. `312%`).
Cap display at `999%` to avoid layout overflow.

### New `process_tree.rs` module

Extract the BFS logic into `agentmuxsrv-rs/src/backend/blockcontroller/process_tree.rs`
rather than inlining it in `sysinfo.rs`. Keeps sysinfo.rs focused on scheduling/publishing.

```
agentmuxsrv-rs/src/backend/
  blockcontroller/
    pidregistry.rs      (unchanged)
    process_tree.rs     (NEW — collect_descendants())
  sysinfo.rs            (modified — use process_tree, two-pass refresh)
```

Public API of `process_tree.rs`:

```rust
/// Returns root PID + all descendant PIDs, capped at `max_pids`.
pub fn collect_descendants(sys: &sysinfo::System, root: Pid, max_pids: usize) -> Vec<Pid>;
```

---

## Frontend Changes

**None required.** The `blockstats` event payload already carries `values.cpu` and
`values.mem`. The `BlockStatsBadge` and `useBlockStats` hook are unchanged.

Optional enhancement: display `values.pids` as a small count badge
(e.g. `312% 1.2G ×4`) to signal that 4 processes are being tracked. Deferred.

---

## Performance Budget

| Operation | Estimated cost | Notes |
|-----------|---------------|-------|
| Minimal full-process refresh (step 1) | ~0.5–1ms | No CPU accounting |
| BFS per registered block | <0.1ms | ~300 nodes typical |
| Targeted deep refresh (step 3) | ~1–3ms | Same as current but more PIDs |
| Total added latency vs current | ~1–2ms | Within 1s tick budget |

The key insight: step 1 (cheap full refresh) is what makes step 2 possible. Without it,
we have no parent→child map. The two-pass approach keeps the expensive CPU accounting
targeted to only the PIDs we care about.

**Worst case:** a user with 50 registered blocks each with 64-PID trees = 3200 PIDs in
the targeted refresh. Still well within sysinfo's capacity. The 64-PID cap per block
prevents runaway.

---

## Edge Cases

### PID reuse
If a shell exits and a new unrelated process claims the same PID before the registry is
updated, we'll briefly attribute wrong metrics. Mitigated by `unregister()` being called
on controller shutdown. Acceptable — same race exists in every process monitor.

### Process exits mid-tick
A PID in the descendant set may exit between step 1 and step 3. `sys.process(pid)` will
return `None` after `remove_dead_processes = true`. The loop skips it. No panic.

### Orphaned descendants
If the shell (root) exits but a grandchild is adopted by init/PID 1, BFS from the dead
root finds nothing. Metrics drop to zero. Correct behavior.

### Windows PID tree
sysinfo 0.34 provides `Process::parent()` on Windows via `CreateToolhelp32Snapshot`.
Works identically to Linux/macOS for our purposes.

### Agent pane
The agent controller (`cmd` type) spawns the CLI as a child of the shell. With tree
tracking, the agent CLI's CPU is automatically included. No special case needed.

---

## Files to Modify

| File | Change |
|------|--------|
| `agentmuxsrv-rs/src/backend/blockcontroller/process_tree.rs` | **NEW** — `collect_descendants()` |
| `agentmuxsrv-rs/src/backend/blockcontroller/mod.rs` | Export `process_tree` module |
| `agentmuxsrv-rs/src/backend/sysinfo.rs` | Two-pass refresh, use `process_tree` |

Frontend: no changes.

---

## Success Criteria

- Running `cargo build` in a terminal pane shows >0% CPU during compilation
- Running `claude -p ...` in an agent pane shows the CLI process CPU
- Shell at idle shows ~0% (not inflated by unrelated system activity)
- Badge updates every 1s (unchanged from current)
- No panic or error logs when a process exits during a tick
- Single pane with 8 parallel workers shows sum >100% (correct, expected)
- Total sysinfo loop duration (logged at trace level) does not exceed 10ms per tick

---

## Non-Goals

- Tracking GPU per pane (separate issue)
- Historical CPU sparkline (separate issue)
- Per-thread breakdown
- Killing processes from the badge
