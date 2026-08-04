# Retro: a throwaway `task dev` test build resumed a live production agent session and spawned an unaware second turn

**Date:** 2026-07-29
**Severity:** High (a second, independent agent process edited the same shared git working directory as the live production process, with neither aware of the other; only benign by luck of what the injected instruction happened to be)
**Affected versions:** since P0.2/P0.3 made the agent registry/definitions/transcript store channel-independent (pre-dates this retro; exact PR not identified in this investigation).

---

## What happened

While live-verifying a CSS/scroll-follow fix (PR #2349), I launched a throwaway `task dev` build on branch `agent3/scrollbar-stacking-and-follow-hardening` purely to visually confirm the fix in a running window via CDP. I opened its agent picker, clicked "Continue Agent3" expecting an old, disposable test session, and typed a synthetic message ("run this in a shell tool: a harmless echo/sleep loop") to trigger the UI states I needed to inspect (the "Working…" row, `ActivityDock`).

The picker's "Agent3" entry was not a disposable test session — it was **the live, currently-running production conversation** (this one). Submitting the message queued a real user turn against it. Roughly ten minutes later, after PR #2349 had already merged, `git status` in the shared working directory showed an uncommitted edit to the exact file I'd just shipped, in my own commenting voice, describing "live CDP-driven testing" that found a third root cause for the same scroll-follow bug — work I never wrote. A second, independent agent process had picked up the injected message, inherited the full standing context of the scrollbar task (because it was resuming the *same* session transcript), and autonomously kept going: it used the Bash tool against the same real git checkout to investigate and patch the same file, unaware that a different process (this one) already owned and had just shipped that work.

The injected instruction itself was harmless (an echo/sleep loop). Had it been anything with real side effects, two unaware agent processes would have been acting on the same repository, credentials, or external systems concurrently.

This went further than a file-content collision. After my PR merged, `git reflog` on the shared working directory showed the second process had run an actual `git checkout` in that same physical checkout — `HEAD@{3}: checkout: moving from agent3/scrollbar-stacking-and-follow-hardening to agent3/scroll-follow-overflow-anchor-gating` — creating and switching to its own new branch for its follow-up fix, silently changing the branch my own concurrent shell commands were operating against. It followed the correct workflow for its own task (pull latest main, branch off it, commit locally) — the danger isn't that its process was misbehaving, it's that **two independent processes were running that workflow against the same physical directory at the same time**, so a `git checkout`/`git stash`/`git reset` from either side could land while the other was mid-command. This time it only caused a confusing branch swap and a diff I had to stash; an unluckier interleaving (e.g. one side's `git checkout` firing while the other has staged-but-uncommitted changes) could discard work outright.

## Root cause

Traced end-to-end (frontend → RPC → storage) by an Explore subagent, then spot-checked directly:

- The picker (`frontend/app/view/agent/components/MyAgentsList.tsx`) is populated by `RpcApi.ListRecentSessionsCommand`, handled server-side in `agentmux-srv/src/server/agent_handlers/session.rs` (`COMMAND_LIST_RECENT_SESSIONS`). It reads from `wstore.shared_agent_registry()`.
- That registry, plus agent **definitions** and conversation **transcripts**, are NOT resolved via the per-build/per-channel `data_dir` (`RuntimeMode`-scoped: `~/.agentmux/dev/<branch>/<clone_id>/…` for dev builds, `~/.agentmux/channels/<channel>/versions/<v>/…` for portable/installed builds). They resolve via `agentmux-srv/src/registry/paths.rs::resolve_global_shared_root()`, which falls back to a single, OS-home-scoped `~/.agentmux/shared/agents/{registry,definitions,transcripts}` — **deliberately independent of channel/build isolation.** The doc comments on `resolve_shared_registry_dir`/`resolve_shared_definitions_dir` explain this was intentional (P0.2/P0.3): "an agent created/named in one channel is visible in every channel," "a conversation loads when you open the agent from any build/channel."
- Clicking "Continue" (`agent_open.rs`) seeds `resume_session_id` from that same global registry when no live controller for that session exists **in the current process**. It has no way to know a *different* srv process (my real production instance, running as a separate `RuntimeMode::Portable` build) already owns that session live.
- Submitting a new message then spawns a brand-new local child process (`agentmux-srv/src/backend/blockcontroller/core.rs`, `tokio::process::Command`) inside whichever srv process received the RPC — in this case, the dev sidecar's own srv, entirely locally, with no coordination with or awareness of the other srv process.

In short: **agent identity/session data is deliberately global (by design, for good reason — so a named agent and its history follow you across builds/channels), but turn ownership/execution is per-process and has no cross-process lock or liveness check.** Any two AgentMux processes on the same machine that resolve the same `~/.agentmux/shared` root can both believe they're free to resume and drive the same agent identity's next turn, and both can freely read/write the same real filesystem (agent shell tool calls run in the actual working directory the session was opened against — not something virtualized per data_dir/build).

This is not a bug in the shared-registry design itself (cross-channel agent persistence is a deliberate, sensible feature) — it's a missing invariant: nothing enforces "at most one live process may drive a given session's next turn at a time."

## Why it wasn't caught sooner

`task dev` test builds are visually and data-directory isolated enough (separate CEF window, separate srv process, separate port, separate `data_dir`) that treating one as a safe, disposable sandbox is the obvious and previously-untested assumption. The isolation is real for local app state (settings, window layout, per-build caches) — it just doesn't extend to the shared agent registry/transcripts, and nothing in the dev-mode UI signals that distinction (the picker looks identical whether the session it's about to resume is a stale historical one or a currently-live one). This is the first time a `task dev` instance was used to interact with a real, named agent identity rather than a scratch/never-before-seen one, so the gap had no prior occasion to surface.

## Fix

None shipped as part of this retro — this is a process/tooling gap, not a defect in the scrollbar PR that prompted its discovery. Filed as follow-up work (see below) rather than fixed inline, since a real fix (session-level locking or ownership across processes) is a meaningfully-sized backend change deserving its own design and review, not a rider on an unrelated frontend PR.

## Explicit follow-ups (not fixed here)

- **Cross-process turn ownership.** The core gap: no mechanism stops two srv processes from both resuming and driving the same session concurrently. A durable lock/lease on `session_id` (e.g., a row in the shared registry recording the owning process's PID + heartbeat, checked and refused-if-held on resume) would close this at the source.
- **Dev-mode picker safety.** At minimum, the picker should visibly distinguish or refuse to resume a session that has a live owning process elsewhere (once the above lock exists, this is a natural byproduct: refuse/warn instead of silently spawning a second driver).
- **This agent's own practice going forward:** never resume a real, named agent identity's session for throwaway UI testing. Live CDP verification of dev builds should either (a) create a fresh, never-before-named test agent identity, or (b) restrict interaction to purely visual/geometric checks (as was done for the rest of this task after the incident) rather than submitting messages that create real turns.
- **Shared physical working directory as a hazard independent of the above.** Even with cross-process turn ownership fixed, this machine's agents (this session included) routinely run their own `git checkout`/branch/commit workflow directly against one shared clone (`C:\Users\asafe\agentmux`), not an isolated worktree per agent/task. That's fine when only one process is actively mutating it, but this incident showed two processes doing so concurrently, unprompted by either operator. Worth a separate investigation into whether concurrent agent work in this environment should default to `git worktree`-per-task rather than sharing one checkout — this retro only surfaces the risk, it doesn't resolve it.

## Lessons

1. **"Isolated build" is not the same as "isolated state."** A dev build having its own process, port, and data directory strongly suggests full isolation; verifying which specific subsystems are and aren't covered by that isolation (here: the shared agent registry/transcript store, by explicit design) matters before treating it as a safe sandbox for real interactions.
2. **Deliberately global state needs an equally deliberate ownership story.** Making agent identity/history follow the user across builds/channels was a good, intentional design choice — but it was designed for the single-live-process case. Extending "global" to session *data* without also deciding what happens when two processes can both act on that data left an unguarded seam.
3. **When something you didn't write shows up in your own working tree, stop and investigate before touching it.** The uncommitted diff looked enough like legitimate in-progress work (plausible reasoning, correct comment conventions, matching the exact bug class) that discarding or overwriting it without investigation would have destroyed a real, independently-arrived-at finding.
