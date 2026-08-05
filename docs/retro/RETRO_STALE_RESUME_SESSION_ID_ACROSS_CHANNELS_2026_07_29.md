# Retro: first message to a globally-known agent from a brand-new build/channel failed with a generic "Agent encountered an error"

**Date:** 2026-07-29
**Severity:** Medium (message is lost and must be resent; the agent recovers on the very next send, but the failure is visible and unexplained)
**Affected versions:** any persistent-controller agent (`agentmux-srv/src/backend/blockcontroller/persistent.rs`), any time its session is first resumed under a build/channel whose CLI install has never locally seen that session. First observed on the freshly-built `0.54.7+g38cc9ac99` portable, immediately after PR #2338 (fast-fail-send-while-unauthenticated) merged.

---

## What happened

Live-tested the fresh `0.54.7` portable build. Logged into the "Lzop" agent successfully (`✓ Login successful`), then sent the message `u there`. The agent immediately failed with a generic **"Agent encountered an error"** banner — no response, message effectively lost.

This is **not** a regression from PR #2338 or any of its ~39 review rounds. That PR's entire review surface was the ACP-controller's `outstanding_prompt_ids` tracking, `useAgentCommands.ts`'s deferred-controller-refresh machinery, and the frontend fast-fail auth guard — all of which sit in front of, and are agnostic to, the controller-type-specific code that actually failed here. "Lzop" is a **persistent**-type controller (`persistent.rs`), and the failure is in that file's session-resume path, a codepath PR #2338 never touched. The user *did* log in successfully and *was* authenticated — the auth guard worked exactly as designed; the failure is a completely separate, pre-existing gap in an unrelated subsystem.

## Root cause

Traced via `~/.agentmux/logs/agentmuxsrv-v0.54.7.log.2026-07-29`:

1. On pane open, the backend backfills the pane's subagents from a session id already on record for this agent: `session_id: 2b75fd90-e509-4b09-b98f-6a190d6c707e` (`subagent_watcher::scan`, "backfilling session subagents on pane (re)open").
2. Sending `u there` spawns the persistent CLI with `--resume 2b75fd90-e509-4b09-b98f-6a190d6c707e` against the **v0.54.7 channel's own, brand-new `npm install`** of `@anthropic-ai/claude-code` at `~/.agentmux/instances/v0.54.7/cli/claude` — a directory that had never run before this exact build.
3. The CLI process immediately writes to stderr: `No conversation found with session ID: 2b75fd90-e509-4b09-b98f-6a190d6c707e`, then **exits with code 1** — it never produces a response to `u there` at all.
4. `persistent.rs`'s stderr reader recognizes this exact line (this is a known, previously-encountered failure mode — see the code's own extensive comments referencing `SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md §4.2`), clears the stale session id (`poison_resume` + `persist_session_id(..., "")`), and calls `session_recovery::mark_resume_failed`, which sets `session:resume_failed` on the block so `AgentControlBar.tsx` can show a distinct disclosure banner ("Couldn't resume the previous conversation — started a new one.").
5. But this recovery only prepares the **next** send to succeed (fresh conversation, no `--resume` flag). The **current** send — the message the user actually typed — was already lost when the process exited with code 1. That bare non-zero exit with no output falls through to the generic fallback error string, which is literally `"Agent encountered an error"` (`agentmux-srv/src/agents/translator/claude.rs:299`, mirrored in `frontend/app/view/agent/providers/claude-translator.ts:84`) — exactly what appeared.

The deeper architectural cause (documented the same day in a sibling investigation, `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`): **agent identity, definitions, and conversation transcripts are deliberately global** — resolved via `resolve_global_shared_root()` under `~/.agentmux/shared/...`, independent of build/channel, specifically so an agent and its history follow the user across every build. But the actual CLI **binary + its own native session/project storage** are provisioned **per version/channel** (`~/.agentmux/instances/v<version>/cli/claude`, a fresh `npm install` per channel, confirmed in this log by `"running npm install"` firing on first use of v0.54.7). A session id recorded while the conversation last ran under a *different* channel's CLI install is therefore unreachable the first time it's resumed under a brand-new channel — the global "this agent's last session" pointer and the per-channel "what conversations does this CLI installation actually know about" state can disagree, and nothing checks that they agree before spawning with `--resume`.

This is the same root shape as the sibling retro (global agent state vs. a boundary-scoped execution layer that isn't aware of it) but manifests in a different layer: that retro was about **cross-process turn ownership** (two live srv processes resuming the same session concurrently); this one is about **cross-channel CLI-install session reachability** (one srv process, but a session id that only one specific channel's CLI install can actually resolve).

## Why the existing mitigation didn't fully cover this

The `resume_failed` disclosure mechanism (§4.2) was *already built* for exactly this failure signature (the code comment predates this incident: "a permanent 'Agent encountered an error' with no path to recovery" was the exact scenario it exists to close for *future* sends). It does close the loop for every send **after** the first — the poisoned session id is cleared, so the next message starts a fresh conversation and succeeds normally. The gap is narrower than "no recovery at all": it's that the **triggering send itself** still visibly fails and its content is lost, because the CLI process has to actually exit before the stderr line proving "this session is unreachable" can be observed — there's no way to know in advance, before spawning, that this particular `--resume` will fail.

## Fix

None shipped as part of this retro — this is a pre-existing gap in a different subsystem than the one just reviewed, not a defect in PR #2338 or the v0.54.7 release. Filed as follow-up rather than fixed inline, matching this session's own established practice of not bundling unrelated architectural fixes onto a merged/shipped change.

## Explicit follow-ups (not fixed here)

- **Don't lose the triggering message.** When `persistent.rs`'s stderr reader detects `"No conversation found with session ID"` on the very first spawn attempt for a message, it already knows this exact failure is recoverable by respawning without `--resume`. Instead of letting the process exit and surfacing a generic error, it could transparently retry the same send once, without `--resume`, before ever reporting failure to the user — turning this into a silent (or disclosed-but-non-error) fresh-conversation start rather than a lost message the user must notice and resend.
- **Validate reachability before spawning, not after.** If there's a cheap way to check whether a given channel's CLI install actually has a given session id on record (e.g. a lightweight local lookup) before ever passing `--resume`, the stale case could be detected and handled pre-spawn instead of via a stderr-pattern-match post-mortem.
- **Reconcile with the sibling retro's proposed fix.** If cross-process turn ownership ever gains a durable per-session lock/lease (that retro's proposed fix), consider whether the same lease record could also carry "last channel this session actually ran under," letting a new channel's first resume attempt know in advance that a fresh conversation is required, rather than discovering it via a failed spawn.

## Lessons

1. **A merged PR's review scope is not the whole app's error surface.** ~39 rounds of adversarial review on the auth-guard/deferred-refresh machinery correctly hardened *that* subsystem — but a completely different controller type's completely different session-management code was never in scope, and a first-use-of-a-new-channel failure mode there was always going to surface independently of how thorough that other review was.
2. **"Global by design" state needs a matching-scope reachability check.** This is the second same-day finding (after the sibling retro) where a deliberately-global piece of agent state (identity/session pointer) outran the awareness of a narrower-scoped layer (a live process; a per-channel CLI install) that has to actually act on it. Both times, the global state was the right design choice — the gap was assuming the narrower layer could always successfully act on whatever the global layer handed it.
3. **A "first attempt after this" fix isn't a "no user impact" fix.** The `resume_failed` mitigation is genuinely good engineering — it closes an infinite-failure loop — but recovering by the *second* attempt still means the *first* attempt visibly fails from the user's point of view. Worth distinguishing, when writing a retro or judging a fix's completeness, between "this can't happen forever" and "this never happens."
