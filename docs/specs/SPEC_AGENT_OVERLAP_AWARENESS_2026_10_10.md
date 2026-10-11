# SPEC: agents can see who else is working on the same thing

**Status:** active (phase 1: #4630; phases 2 and 3 to follow)
**Date:** 2026-10-10
**Asked by:** the owner: "agents may be working on the same thing … an API to verify they aren't stepping on anyone else's toes."

## 1. The problem

Several agents often work in the same repositories at once. Each has its own clone, so they never collide on disk;
they collide later, on GitHub: two PRs that change the same files, a fix built twice, one agent's change reverting
another's. Today an agent avoids that only by messaging the others by hand and reading their branches. This week, every
cloud change by one agent began with "I'm touching these files, are you?" sent to two others.

What an agent can learn about other agents today (all on this computer, across channels, through MCP):

| Tool | What it says about the others |
|---|---|
| `DiscoverAgents` / `FleetList` | names, ids, the folder each started in |
| `ListConversations` | busy or idle, last activity, a 200-character preview of the transcript |
| `GetAgentTranscript` | the raw transcript, to read through |
| `WorkClaim` and the rest of the work queue | claims with leases, but on queue items, not on files, branches or PRs; agents almost never use it |

What it can't learn: another agent's goal, todo list, working folder, repository and branch, uncommitted files, files it
edited recently, or open PRs. The Swarm view shows the human most of this (goal, todos, current tool, status), but
only in the UI.

## 2. Goals

1. **Ask:** one MCP call answers "who else is working on this path / repository / branch / PR / topic, and how".
2. **Be told:** an agent learns about an overlap without having to ask: when it first edits a file another agent
   has uncommitted or recently edited, and when it opens a PR whose files another agent's open PR also changes.
3. **Claim (optional):** an agent can say "I'm taking this" for a path, branch or topic, so others see a claim, not
   just activity.

Non-goals: blocking anyone (this informs, it doesn't lock); sharing work content beyond this computer (LAN and cloud
peers keep seeing names and status only, as decided for the fleet feed); replacing talking to each other (the answer
names the agent to message).

## 3. Design

### 3.1 Work facts per agent (srv)

srv keeps, per running agent, a small set of facts it can already see or cheaply derive:

| Fact | Source |
|---|---|
| goal | the generated title (`term:ambient_summary`), else the last prompt (`term:last_prompt`) |
| todos, current tool | what the progress watcher already parses from the agent's output for the Swarm (`backend/reactive/progress_watcher.rs`) |
| recent files | the same watcher, keeping the paths from Edit / Write / MultiEdit / NotebookEdit tool inputs (bounded, newest first, with times) instead of dropping them |
| working folder | the agent's live cwd (the per-agent cwd state file bashwrap already writes), else its start folder |
| repository, branch, uncommitted files | `git` in that folder: the remote URL (normalized `owner/repo`), `branch --show-current`, `status --porcelain` (cached, refreshed at most every 30 s per agent, never on the request path) |

Files are compared across clones by **(repository, path inside the repository)**, since every agent has its own clone.

### 3.2 Ask: `WhoIsWorkingOn`

A new MCP tool, and the same facts added to `ListConversations` entries:

```
WhoIsWorkingOn({ path?, repo?, branch?, pr?, query? }) →
  [{ agent, channel, status, goal, repo, branch,
     matches: [{ kind: "dirty" | "edited" | "branch" | "pr" | "claim" | "goal", detail, since_ms }] }]
```

- `path` may be a file or a folder (prefix match), absolute in the caller's clone or repository-relative; the
  caller's own repository is assumed when `repo` is omitted.
- `query` matches goals and todos (case-insensitive words), for "is anyone already doing the MuxBus allowlist?".
- The caller itself is left out. Results are ordered by strength: claim, uncommitted, edited, branch, pr, goal.
- Reach: every agent on this computer, all channels (the same fan-out `ListConversations` already does). Agents on
  other computers appear by name and status only, with no matches.

### 3.3 Be told: overlap notes

srv checks for an overlap when an agent's watcher sees it edit a file, and when it runs a PR create:

- **File:** the same (repository, path) is uncommitted or edited in the last 2 hours by another agent →
  one AgentMux system note (the existing `[AgentMux]` note, which no jekt can forge):
  `[AgentMux] Agent4 also has uncommitted changes in crates/srv/src/muxbus/presence.rs (repo agentmuxai/agentmux, branch agent4/presence). Message them before changing it.`
- At most one note per (other agent, file) per 2 hours, and at most 3 notes per agent per 10 minutes (then one
  summary). No note for the agent's own other clones.
- Phase 2 as built: only edits the watcher reads live are checked (never a backlog read after a restart), each
  (pane, file) at most once a minute. The check runs on its own task, with the other channels' facts fetched under
  the same per-channel timeout as `WhoIsWorkingOn`, so the watcher never waits. Edits of the same file by agents of
  the same name (another clone, pane or channel) are never reported. One note names every other agent on that file;
  the fourth note in an agent's 10 minutes is the summary (`… 4 files you edited in the last 10 minutes are also
  being changed by other agents; the latest is …. No more of these notes for N minutes: … call WhoIsWorkingOn …`),
  then nothing until the window ends. The note goes to the editing agent only. Off per install with the setting
  `agent:overlapnotes: false` (default on).
- **PR:** on `gh-agent pr create` / `gh pr create`, srv lists the other agents' open PRs in the same repository whose
  changed files intersect this branch's, and notes them once.

### 3.4 Claim

`ClaimWork({ path? | branch? | topic, note, ttl_minutes = 120 })` and `ReleaseWork`. Claims use the work queue's
existing lease store (host-wide, across channels, expiring), so a crashed agent's claim lapses by itself. A claim shows
in `WhoIsWorkingOn` (kind `claim`) and in overlap notes ("Agent5 claimed crates/srv/src/muxbus/ 40 min ago: presence
work"). Claims never block.

### 3.5 Open PRs

PRs come from GitHub, so they work across computers. `WhoIsWorkingOn({ pr })` and the PR note read open PRs through
the caller's `gh-agent` (the agent's own GitHub identity; srv holds no GitHub credentials) and attribute them by the
`<Agent>@<host>:` title prefix that CI already requires. Results are cached for 2 minutes.

## 4. Phases

| Phase | What |
|---|---|
| 1 | Work facts (3.1); `WhoIsWorkingOn` without PRs (3.2); the facts on `ListConversations` |
| 2 | Overlap notes for files (3.3, file half) |
| 3 | Claims (3.4); open PRs in `WhoIsWorkingOn` and the PR note (3.5, 3.3 PR half) |

Each phase is one PR. Phase 1 is useful alone: an agent can check before starting.

## 5. Testing

- Facts: a fake agent output stream with Edit/Write calls fills recent files; a temp git repo gives repository,
  branch and dirty files; the 30 s cache is respected.
- `WhoIsWorkingOn`: two fake agents with clones of the same remote in different folders match on repository-relative
  paths; prefix matching; `query` over goals and todos; the caller is excluded; cross-channel results.
- Notes: rate limits per pair and per agent; no note for one's own clone; the note text names agent, file, repo,
  branch.
- Claims: lease expiry, release, visibility across channels.
