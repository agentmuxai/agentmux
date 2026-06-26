# Spec: Local Build Channel Pruner

**Date:** 2026-06-25
**Status:** Draft
**Author:** oozp-0621f
**Related:**
- `docs/specs/SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md` §6 — tracked follow-up
- `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` — I1–I6 invariants
- `docs/analysis/ANALYSIS_SYSTEM_MEMORY_2026_06_25.md` — incident that prompted this spec
- `docs/retro/` — no dedicated retro yet; see analysis above

---

## Problem

Each `task package` / `task package:local` build bakes a unique per-build channel
(`local-<branch>-<hash>-<build-id>`) into the binary at compile time. This creates
a fully isolated AgentMux instance — its own named pipe, data dir, and CEF cache —
so launching a new build never joins or kills the previous one.

This is correct behavior for the multi-instance isolation contract (I1–I6). The
**gap** is that there is no mechanism to:

1. **Reclaim disk** from channels whose launcher/host/srv are no longer running
   (dead channels accumulate data dirs + CEF caches indefinitely)
2. **Notify the user** that old live instances are still consuming memory
   (we discovered 731 MB wasted on two 0.48.1 zombies the user forgot about)

The result observed on 2026-06-25: two `agentmux-0.48.1` processes + sidecar
consuming ~731 MB, running for ~30 hours after a newer build was launched.

---

## Non-goals

- **Auto-killing live old instances.** Violates I3 (bounded blast radius) and would
  be hostile UX. A user may intentionally keep two versions running side by side.
- **Pruning `stable` / release channels.** Release installs are long-lived. Only
  `local-*` channels accumulate unboundedly and are safe to target.
- **Pruning channels with data the user might want.** Agent data is global
  (`~/.agentmux/agents/`); layout + memories in `local-*` are per-build ephemeral
  by design. Safe to delete.

---

## Liveness Check — How to Tell Live from Dead

The canonical signal is the **named pipe** (Windows) / **Unix socket** (Unix):

```
Windows:  \\.\pipe\agentmux-{hash16}\command
Unix:     {XDG_RUNTIME_DIR}/agentmux/{hash16}.sock
```

Where `hash16 = fnv1a_64(lowercase(data_dir) + "\x00" + version)` formatted as
16 hex chars (see `agentmux-launcher/src/hash.rs`).

**Best practice (from Chromium, VS Code, Electron single-instance patterns):**

Use **client-connect** (not bind-attempt) as the liveness probe:

| Probe result | Meaning |
|---|---|
| `CreateFile(pipe)` succeeds | Live — launcher is running and accepting connections |
| `ERROR_FILE_NOT_FOUND` | Dead — no pipe server exists |
| `ERROR_PIPE_BUSY` (timeout=0) | Ambiguous — pipe exists but busy; treat as live |

Why connect rather than bind:
- Binding would accidentally "steal" the pipe if the probe runs concurrently with
  a new launcher starting (TOCTOU race)
- Connection is read-only from the OS perspective; no side effects
- `ERROR_FILE_NOT_FOUND` is unambiguous: the pipe server is gone

**Belt-and-suspenders for Windows:** additionally check `OpenProcess(SYNCHRONIZE,
pid)` if a PID is available from the instance_claim log. If the process handle
opens and `WaitForSingleObject(handle, 0)` returns `WAIT_OBJECT_0`, the process
has exited. This handles the edge case where a pipe name was reused by a different
process (extremely unlikely given the FNV-1a hash, but belt-and-suspenders).

---

## Design

### Phase 1 — Passive Dead-Channel Pruner (P0)

**Where:** Inside the launcher, at startup, after the single-instance bind succeeds
(i.e., we are the first instance for this channel). Only runs once per launcher
invocation.

**What it does:**

1. Walk `~/.agentmux/channels/local-*/versions/*/data/` (skip the current channel)
2. For each `(channel, version, data_dir)` triple, compute `hash16`
3. Probe the named pipe/socket with a connect attempt (0ms timeout)
4. If probe → dead AND `data_dir` mtime is older than `PRUNE_GRACE_S` (default: 300s):
   - Delete `data_dir/` and its sibling `cef-cache/` and `logs/`
   - Leave `agents/` and `config/` at the channel root untouched (global data)
   - Log: `channel_pruned { channel, version, data_dir, age_secs, reason: "pipe_dead" }`
5. If probe → live:
   - Record as a live old instance; see Phase 2

**Grace period (`PRUNE_GRACE_S = 300`):** prevents pruning a channel that just died
in the last 5 minutes (crash during startup, etc.). This is the same window the
Unix socket recovery already uses.

**Performance:** The walk + probe should complete in < 50ms for a typical user with
< 20 local channels. Run it in a background Tokio task (don't block the launcher
startup path).

**Disk savings:** Each dead local channel typically holds:
- `data/` — 1–5 MB (SQLite DBs, transcripts)
- `cef-cache/` — 50–200 MB (Chromium user data)
- `logs/` — 1–10 MB

Total recoverable per dead channel: **50–215 MB**. A user who builds daily
accumulates this fast.

---

### Phase 2 — Live Old-Instance Notification (P1)

**Where:** Frontend, at startup, via a new `old_instances` field on the
`AboutModalDetails` RPC response (or a dedicated `list_live_local_instances` RPC).

**What it shows:**

A dismissible banner or notification (not a blocking modal) in the AgentMux window:

```
┌──────────────────────────────────────────────────────┐
│ ⚠  2 older AgentMux builds still running             │
│    0.48.1 · 2 instances · running 31h               │
│                                          [Dismiss]   │
└──────────────────────────────────────────────────────┘
```

- Shown once per session (persisted in session storage, not disk)
- Clicking the version label focuses the oldest live instance's window (via the
  existing `open_new_window` IPC forward path — I4)
- No "Kill" button — let the user decide

**Why not auto-kill:** The user might be running v0.48.1 because a specific agent
session is open there. Auto-killing would destroy that session without warning.
Notification + user action is the right UX for live instances.

---

### Phase 3 — Self-Pruning Offer (P2, future)

Each live launcher periodically (every 10 minutes) checks: "Is there a newer
build of AgentMux running?" by scanning the `local-*` channel pipe names and
comparing build timestamps in the channel name.

If a newer build is detected, the old launcher could:
- Show its own toast: "A newer AgentMux build is running. Close this window?"
- Or silently reduce its memory footprint (close idle panes, flush caches)

This is lower priority — Phase 1 already fixes the disk accumulation and Phase 2
fixes the user-visibility gap.

---

## Implementation Plan

### Phase 1 (launcher Rust — ~2 days)

**Files:**
- `agentmux-launcher/src/pruner.rs` (new) — dead-channel discovery + deletion
- `agentmux-launcher/src/main.rs` — spawn pruner task after single-instance bind

```rust
// agentmux-launcher/src/pruner.rs (sketch)

pub async fn prune_dead_local_channels(
    current_channel: &str,
    channels_dir: &Path,
) {
    let Ok(entries) = tokio::fs::read_dir(channels_dir).await else { return };
    // ... walk local-* subdirs, for each compute hash16, probe pipe,
    // if dead + old enough: delete data/ cef-cache/ logs/
}

fn is_pipe_alive_windows(hash16: &str) -> bool {
    let pipe = format!(r"\\.\pipe\agentmux-{}\command", hash16);
    // CreateFile with OPEN_EXISTING, timeout=0, no wait
    // ERROR_FILE_NOT_FOUND → false (dead)
    // success or ERROR_PIPE_BUSY → true (live)
    ...
}
```

**Launch site (main.rs):** after `bind_socket_with_recovery()` / named pipe bind
succeeds, before the saga coordinator starts:

```rust
let channels_dir = data_paths::channels_dir(&home);
let current = channel.to_string();
tokio::spawn(async move {
    pruner::prune_dead_local_channels(&current, &channels_dir).await;
});
```

### Phase 2 (frontend + srv RPC — ~1 day)

- Add `ListLiveLocalInstances` RPC to `agentmux-srv/src/server/app_api.rs`
- Backend: probe all `local-*` channels (same liveness logic, but returns live ones)
- Frontend: call on startup, show dismissible notification banner if non-empty
- Reuse existing notification system (`setNotifications`) — notification type:
  `"old-instances"`, persistent: false

---

## Best Practices (prior art)

| App | Approach |
|-----|----------|
| **Chromium** | Client-connect probe on startup; if alive, send `--new-window` via IPC and exit; if dead, bind and proceed. Named pipe on Windows, Unix socket otherwise. |
| **VS Code** | `requestSingleInstanceLock()` (Electron API) + stale workspace cleanup on startup: scans `~/.vscode/`, deletes lock files whose PIDs are no longer alive. |
| **Electron** | `app.requestSingleInstanceLock()` — tries to connect to existing instance's IPC; if timeout → assume dead; bind and proceed. |
| **JetBrains IDEs** | Per-project `.idea/workspace.xml` lock with PID; on startup, if PID is not alive → remove lock. Also scans for "recent projects" whose data dirs are stale. |
| **Firefox** | `parent.lock` file in profile dir containing PID; on startup, `kill(pid, 0)` → if ESRCH: stale, remove and proceed. |

**Common pattern across all:** connect-probe first (non-destructive), fall through to
bind on failure. Delete stale artifacts only after confirming the previous holder is
gone. Grace period to handle concurrent startups.

AgentMux's approach (named pipe bind as single-instance gate) is already aligned with
Chromium's model. The pruner simply adds the "scan all local channels and probe each"
pass that these other apps do for their equivalent of "stale workspaces / old profiles."

---

## Edge Cases

| Case | Handling |
|---|---|
| Two new builds start simultaneously | Each probes the other's pipe → both see it as "live" → neither prunes the other. Correct. |
| Build crashed mid-startup (pipe never bound) | `data_dir` mtime is very recent → grace period protects it. After 5 min it's prunable. |
| User has `AGENTMUX_CHANNEL` env var pointing to a local channel | Data dir is still found by the walk; probe the pipe; if dead, prune. Env var only affects the current instance, not others. |
| `cef-cache/` is very large (several GB) | `tokio::fs::remove_dir_all` is async; wrap in `tokio::task::spawn_blocking` for large deletes to avoid blocking the async runtime. |
| NAS / slow FS | Walk may time out. Add a 5s timeout on the entire pruner task; if it times out, log and skip. Don't block the launcher. |
| Windows: pipe name collision (different machine, same hash) | Impossible — hash includes canonical lowercase data_dir which is machine-local. |

---

## Success Metrics

- **Disk:** `local-*` channels older than `PRUNE_GRACE_S` with no live pipe are
  deleted automatically on next launch. No manual `rm -rf` needed.
- **Memory:** User is notified of live old instances within 5s of launching a new
  build. Notification includes uptime so they can make an informed choice.
- **Safety:** Zero incidents of a running instance's data being deleted (grace period
  + liveness check belt-and-suspenders).
- **Latency:** Pruner completes in < 100ms for ≤ 50 local channels. Does not delay
  the splash screen or first window open.
