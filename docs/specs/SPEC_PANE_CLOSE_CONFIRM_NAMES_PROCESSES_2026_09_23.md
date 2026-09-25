# SPEC: The pane-close confirmation names the processes it will stop

**Date:** 2026-09-23
**Status:** proposed
**Author:** Korp@narko
**Related:** `docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md` §4.6
(the confirmation this spec extends), `docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md`
(already lists the same processes in the Shell drawer), #3430 (`agent_started`: count only
processes the agent started through a shell).

---

## 0. The ask

> when I close your pane it says there are 2 processes running. we want to update it so it
> shows what the processes are

## 1. What happens today

Closing a pane that holds a busy agent shows:

> **Close this pane?**
> Closing stops these agents. A turn in progress is interrupted, and processes they started are stopped.
> • Korp — 2 processes running

The user has to decide whether to close without being told what would be killed. It could be a
dev server they want to keep, a stray `sleep`, or the agent's own tooling.

### 1.1 Code path

| Step | Where |
|---|---|
| The pane × calls `beforeNodeDelete` | `frontend/app/tab/tabcontent.tsx:122` |
| It collects busy members | `busyMembers()` in `frontend/app/tab/pane-close-guard.ts:39` |
| The process probe keeps **only the length** | `paneCloseProbe.processCount` in `tabcontent.tsx:41-42`: `(await RpcApi.AgentProcessListCommand(...)).processes.length` |
| Each agent is rendered as one line of text | `describeBusyMember()` in `pane-close-guard.ts:53` |
| The backend list | `agent.process-list` in `agentmux-srv/src/server/app_api/agent_io.rs:15`, backed by `AgentProcessRegistry::list_block`, which calls `agent_started()` (`agentmux-srv/src/backend/process_tracker/mod.rs:94`) |

**The data already exists.** `agent.process-list` returns `pid`, `command` (the full command
line), `rss_bytes` and `started_at_ms` for every process, plus a `confidence` value. The dialog
throws all of it away except the count. The Shell drawer's info panel
(`AgentShellInfoPanel.tsx`, `useTrackedProcesses.ts`) already renders the same list, with names,
PIDs and memory.

### 1.2 What was measured while debugging this (2026-09-23, Windows, 0.56.12)

I dumped Korp's own process tree (`claude.exe` and its descendants) and checked it against
`agent_started`'s rules (#3430, which is in the 0.56.12 build that was running):

- **Idle:** `claude.exe` → `conhost`, `agentmux-mcp` → `conhost`. **0 counted.** The filter
  works: the CLI, its conhost and MCP servers are excluded.
- **One foreground Bash tool call** (Claude Code → `agentmux-bashwrap`): **about 7 counted**, and
  every one is harness plumbing around the single command the agent actually ran:
  `bash -c "source …snapshot…"` → `bash` → `bash` → `agentmux-bashwrap exec …` → `bash -c "{ … }"` → `bash` → the command.
  In a dialog listing processes one per line, this would show as seven rows, six of them noise.
- **A background tool call** (`run_in_background`) was **not** under `claude.exe` in a
  parent-PID walk while it ran. Whether it's still in the block's Job Object couldn't be checked
  from outside srv (job membership isn't visible to `Get-CimInstance`). See open question §7.1.
- **PID reuse is real on this machine:** `AMDRSServ.exe`, started the previous day, showed up as
  a "child" of a bash started seconds earlier, because Windows had reused its old parent's PID.
  `agent_started` only walks inside job membership, so its count isn't affected. A UI that builds
  a tree from `parent_pid` **is** affected (§4.2).
- **The user's "2 processes" couldn't be identified after the fact.** When I looked, Korp had no
  leftover orphans, and the only orphan on the machine (`gh-agent.sh pr checks 3589`) belonged
  to another agent. The confirmation is the one moment these processes are visible to the user,
  and nothing about them was recorded. That's the second half of this spec (§4.5).

## 2. Goals

1. The confirmation says **what** each busy agent is running, in terms a user recognizes:
   `npm run dev`, `sleep 75`, `node server.js`, not `bash.exe (4476)`.
2. The harness's shell wrapper chains collapse into the one command they wrap.
3. What's shown is bounded: a pane with 40 processes must not produce a 40-row modal.
4. A close that was confirmed or cancelled leaves a record in the app log, naming the
   processes, so a later "why did it say N?" can be answered.
5. No new failure mode: a probe that fails still counts as "not busy" (§4.6 of the parent spec's
   rule: a broken lookup must never make a pane impossible to close).

## 3. Non-goals

- Changing **which** processes count. `agent_started`'s membership rule (#3430) stays. If the
  count itself is wrong, the named list will now show that, and fixing it is a separate change.
- Per-process kill buttons in the confirmation ("close the pane but keep the dev server"). The
  kill RPCs exist (`agent.kill-process`, `agent.kill-tree`), but "keep" conflicts with the
  parent spec's "tracked ⇒ dies with the pane" contract. See §8.
- Changing the tab-close confirmation (`tab-close-confirm-modal.tsx`). It doesn't mention
  processes today.

## 4. Design

### 4.1 Backend: add `parent_pid` to the RPC row

`AgentProcessInfo` (`agentmux-srv/src/backend/rpc_types/agent.rs:215`) mirrors `TrackedProcess`
but leaves out `parent_pid`, which the tracker already fills in. Add it:

```rust
pub struct AgentProcessInfo {
    pub pid: u32,
    pub command: String,
    pub rss_bytes: u64,
    pub started_at_ms: u64,
    /// Parent PID if the platform exposes it. Lets the UI group wrapper chains (§4.2).
    pub parent_pid: Option<u32>,
}
```

Map it in `register_agent_process_list` (`agent_io.rs`) and add `parent_pid: number | null` to
the inline return type in `frontend/app/store/rpc-api/agent.ts:673`. The field is additive, so
existing consumers (Shell drawer, Swarm) are unaffected.

### 4.2 Frontend: group each agent's processes into chains

New pure function in `pane-close-guard.ts`:

```ts
interface ProcessGroup {
    label: string;            // what the user reads, e.g. "npm run dev"
    pids: number[];           // every member, root first
    rssBytes: number;         // sum across members
    startedAtMs: number;      // the group root's start time
    detail: string;           // full chain for the tooltip, e.g. "bash › agentmux-bashwrap › bash › npm"
}
function groupProcesses(procs: TrackedProcessInfo[]): ProcessGroup[]
```

Rules:

1. **Forest by parent.** A process's parent is `parent_pid` **only if** that PID is in the same
   list **and** its `started_at_ms` ≤ the child's. The second condition is the PID-reuse guard
   from §1.2. Processes with no valid parent in the list are group roots.
2. **One group per root**, holding its whole subtree.
3. **Label = the innermost meaningful command** in the group. Walk down the chain while the
   current node is a *wrapper* and has exactly one non-`conhost` child:
   - a shell (`bash`, `sh`, `zsh`, `cmd`, `powershell`, `pwsh`, the same list as `is_shell` in
     `process_tracker/mod.rs:70`), or
   - `agentmux-bashwrap`.

   Stop at the first non-wrapper, and label the group with that process's command line. If the
   chain is wrappers all the way down (e.g. a shell running a builtin like `sleep` in Git Bash,
   which may be its own process or not), use the innermost shell's `-c` argument.
4. **Command display:** image name plus arguments, whitespace collapsed, truncated to ~80
   characters. The full command line goes in `detail` and the `title` tooltip.
5. **Redaction before display, including the tooltip** (§6): replace token-shaped substrings.

With these rules, the foreground-tool-call chain from §1.2 renders as **one** row:
`sleep 75 · 7 processes`.

### 4.3 What the dialog shows

`BusyMember` gains `processes: TrackedProcessInfo[]` and `groups: ProcessGroup[]`.
`processCount` becomes `processes.length`, so every existing condition still holds. The probe
keeps the list instead of its length.

```
Close this pane?
Closing stops these agents. A turn in progress is interrupted, and processes they started are stopped.

Korp — mid-turn, 2 processes
   npm run dev                        pid 43120 · 2 processes · 212 MB · started 14 min ago
Posa — 9 processes
   sleep 75                           pid 4476  · 7 processes · 71 MB  · started 3 s ago
   node scripts/watch.mjs             pid 50120 · 1 process   · 48 MB  · started 2 h ago
   python -m http.server 8000         pid 50212 · 1 process   · 22 MB  · started 2 h ago
                                                                             + 2 more

                                                              [Cancel]  [Close]
```

- One header line per busy agent, same wording as today. The count stays as a summary, and the
  process rows sit under it.
- **At most 4 group rows per agent**, largest subtree first, then oldest. Any extra rows collapse
  into `+ N more`, which expands in place.
- **Memory** is shown only when `rss_bytes > 0`. **Started** is shown only when
  `started_at_ms > 0`. Reuse `formatBytes` from `AgentShellInfoPanel.tsx` (move it to
  `frontend/util/` rather than duplicating it).
- **Confidence:** if `confidence` isn't `"high"`, add one muted line under that agent:
  `Tracking is best-effort on this platform, so some processes may not be listed.`
- **Mid-turn with 0 processes:** unchanged (`Korp — mid-turn`), with no rows.
- **Layout:** monospace for commands, and the modal stays at its current width (commands
  truncate, they don't wrap). It has to fit a compact-mode window, so it's scrollable past about
  8 rows in total.

### 4.4 Snapshot, not live

The list is the snapshot taken when × was clicked, which is what `busyMembers` does today. It
doesn't update while the modal is open. A process that exits in the meantime is harmless, since
Close kills the tree regardless. One that starts in the meantime is killed without having been
listed, the same as today. It isn't worth a live subscription.

### 4.5 A record in the app log

When the confirmation is answered, log one line per busy agent through `getApi().sendLog`, with
the same text as the modal, untruncated but redacted:

```
[pane-close] confirm=close|cancel block=<id> agent=Korp turn_active=false processes=[43120 "npm run dev" (2), …]
```

This is what would have answered the user's question in §1.2. It's cheap, and it's only
written when a confirmation is actually shown.

### 4.6 Failure behavior

- `agent.process-list` fails → treated as 0 processes (the current `.catch(() => 0)` becomes
  `.catch(() => [])`). The agent is listed only if it's mid-turn.
- `parent_pid` is missing (older srv, or a platform without it) → each process becomes its own
  group, labeled by its own command. That's more rows, but they're still named, and still capped
  at 4 plus `+ N more`.

## 5. Tests

**`frontend/app/tab/pane-close-guard.test.ts`** (extend):
- The §1.2 wrapper chain (bash → bash → bash → bashwrap → bash → sleep) → one group, label
  `sleep 75`, 7 PIDs (conhost excluded).
- Two independent roots → two groups, ordered by size, then age.
- PID-reuse guard: a child whose `parent_pid` matches a list member started *later* than it →
  its own root, not grouped under that member.
- A chain that is wrappers all the way down → labeled by the innermost `-c` argument.
- `parent_pid` null on every process → one group per process.
- More than 4 groups → 4 returned, plus an overflow count.
- Redaction: `ghp_…`, `github_pat_…`, `sk-…`, `Authorization: Bearer …`, `--token=…` are
  masked in both the label and the `detail` string.
- Probe failure → no processes, and the agent isn't busy unless it's mid-turn.

**Rust, `agent_io.rs`:** `agent.process-list` serializes `parent_pid` (present, and `null`).

**Manual:**
- Start `npm run dev` from an agent and close its pane: one row, `npm run dev`.
- Close during a Bash tool call: one row for the command, not seven.
- Cancel the dialog, then check the app log for the `[pane-close]` line.

## 6. Security

Command lines can contain secrets: a token passed on argv, or a `bash -c` script that inlines
one. The data is local to the user who owns the processes (Task Manager shows the same thing),
but a modal ends up in screenshots and bug reports, and the §4.5 log line persists on disk.
So both surfaces redact **values**, not keywords:

- **Known token prefixes:** `ghp_`, `gho_`, `ghs_`, `github_pat_`, `sk-`, `xox[bp]-`, `AKIA…`
- **Header forms:** `Authorization: Bearer <x>`
- **Flags and assignments:** `--token[= ]<x>`, `--password[= ]<x>`, `<NAME>_(TOKEN|SECRET|KEY|PAT)=<x>`

Each value is replaced with `‹redacted›`. The jekt sanitizer's list
(`agentmux-srv/src/backend/reactive/sanitize.rs:155-169`) is a *keyword* list that escalates a
message's tier. It doesn't find or mask values, so it can't be reused here as-is. Keep a small
frontend pattern list, with one test per pattern. Truncating to 80 characters is not a redaction
strategy, because a secret can sit at the start of a command.

## 7. Open questions

1. **Are `run_in_background` tool calls in the job at all?** In §1.2 the background command
   didn't appear under `claude.exe` by parent PID. If Claude Code detaches it (a new process
   group, or a spawn outside the CLI's tree), it may or may not have inherited the job. If it
   didn't, it's never counted, and closing the pane doesn't stop it. Check from inside srv:
   `list_members()` for the block while a background task runs.
2. **Should harness plumbing count at all?** The §1.2 chain is 6 wrappers around 1 command.
   Grouping (§4.2) fixes how it *looks*. Whether `agent_started` should also stop counting
   `agentmux-bashwrap` and the snapshot-sourcing shells is a separate decision under the #3430
   rule, and is out of scope here (§3).
3. **What were the user's "2"?** Unknown (§1.2). Once §4.5 ships, the next occurrence will say.

## 8. Later (not this spec)

- **Per-group Stop** in the confirmation, i.e. "stop this one, keep the pane open". It's safe
  because it doesn't conflict with "tracked ⇒ dies with the pane". It needs its own UX pass.
- **Keep a process across a pane close** (hand it to another pane or detach it). This conflicts
  with `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`, so it needs a spec of its own.

## 9. Files touched (expected)

| File | Change |
|---|---|
| `agentmux-srv/src/backend/rpc_types/agent.rs` | `parent_pid` on `AgentProcessInfo` |
| `agentmux-srv/src/server/app_api/agent_io.rs` | map `parent_pid`, and a serialization test |
| `frontend/app/store/rpc-api/agent.ts` | `parent_pid: number \| null` in the return type |
| `frontend/app/tab/pane-close-guard.ts` | `processes`/`groups` on `BusyMember`, `groupProcesses()`, redaction |
| `frontend/app/tab/pane-close-guard.test.ts` | §5 cases |
| `frontend/app/tab/tabcontent.tsx` | probe keeps the list, renders group rows, `[pane-close]` log line |
| `frontend/util/` (new or existing) | shared `formatBytes`, moved out of `AgentShellInfoPanel.tsx` |
| `.changesets/<ts>-pane-close-confirm-names-processes.md` | `type: patch` |
