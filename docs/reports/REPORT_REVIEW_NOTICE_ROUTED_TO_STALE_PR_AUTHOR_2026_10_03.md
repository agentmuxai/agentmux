# REPORT: a review notice for another agent's PR was delivered to AgentY, because the PR's author is a retired AgentY account

**Date:** 2026-10-03
**Status:** analysis — root cause found and fixes proposed; nothing changed.
**Author:** AgentY, at the owner's request ("look into the false message, looks like a bug")
**Code:** `agentmux-cloud/muxbus/consumers/github/` (`agent-mapping.ts`, `events/review.ts`)
**Related:** `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` (the author-first rule), the "MuxBus Identity" section of the agents' `CLAUDE.md`.

## 1. What happened

At 14:50 UTC on 2026-10-03 AgentY received:

> [ReAgent] PR #448 reviewed — minor notes (non-blocking) — Title: fix(alarms): remove unused RDS alarms and low-value throttle/duration alarms — Branch: `agent2/remove-unused-monitoring-alarms`

PR #448 in `a5af/pulse` is not AgentY's. Its branch is `agent2/…`, its body ends `<!-- agentmux:agent_id=agent2 -->`, and Agent2's own pane the same minute says "Pulse #448: waiting on ReAgent's review. I'll merge it on approval." AgentY did not act on it. The review notice is delivered to the wrong agent, and, by the code below, the right agent does not get it.

## 2. Evidence

PR #448, from the GitHub API:

| Field | Value |
|---|---|
| Created | 2026-07-06 (Agent2 calls it "my stale July PR, rebased") |
| **PR author** | **`AgentY-asaf`** |
| Branch | `agent2/remove-unused-monitoring-alarms` |
| Body tag | `agent_id=agent2` (rewritten with today's changes) |
| Commit `60d1a02e` | git author `AgentY-asaf <253608533+AgentY-asaf@users.noreply.github.com>`, GitHub login `AgentY-asaf` |
| Commit `22bfbb1b` (the head) | git author `Agent2 <agent2@agentmux.local>`, no GitHub login |

The notifier, `processReviewEvent` in `events/review.ts` (lines ~196-225):

1. `prAuthorAgent = getAgentId(pr.user.login)`. `getAgentId` lowercases the name and matches `NUMBERED_PAT_PATTERN = /^agent([xya-g]|[1-5])-[a-z0-9]+$/` (`agent-mapping.ts`). `agenty-asaf` matches, so the author resolves to **`agenty`**.
2. **Only if the author does not resolve** is the `agent_id` tag read (`extractAgentIdFromBody`). Here it resolved, so **the tag naming `agent2` is never looked at**.
3. The head-commit author is then checked and added: `22bfbb1b` has no GitHub login, so it adds nobody.

Result: the notification set is `{agenty}`. Agent2, who owns the PR and is waiting on the review to merge it, is **not** in it. That last part is an inference from the code; I have no access to the consumer's logs or Agent2's inbox, and did not read Agent2's conversation.

## 3. Root causes

1. **Author-first ignores the tag even when they disagree.** `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` made the author win because "a standard agent identity is unambiguous on its own". That holds when the author is the agent doing the work. It fails for a PR whose work was taken over: the author is fixed for the life of the PR, while the tag is the only part anyone can update, and the consumer never reads it.
2. **A retired numbered account still maps to a live agent.** `AgentY-asaf` is a PAT-era account for slot Y. The numbered-pattern rule accepts any `agent<slot>-<anything>`, so a PR opened under an old PAT in July resolves to whoever holds slot Y today, however it came to be authored.
3. **This host's global git identity is `AgentY-asaf`.** `git config --global user.name` reads `AgentY-asaf` with the matching `users.noreply` email. Any agent committing here without its own override authors as AgentY, as `60d1a02e` did. The head-commit check would route a review to `agenty` for any PR whose latest commit was made that way. Agent2 already commits as `Agent2 <agent2@agentmux.local>` where it sets its own, so the convention exists. It is not applied by default, and the machine default belongs to one agent.

(1) alone produced this misroute. (3) is a second path to the same wrong answer that did not fire here only because the head commit happened to carry Agent2's own identity.

## 4. Impact

- **A false message:** AgentY receives reviews of PRs that are not its own. Nothing was acted on here, since the notice asks for nothing. A different notice, for example one that says "merge" or "address this", could be acted on by the wrong agent.
- **A missed message:** the owner agent is not told. Agent2's standing rule is to merge on approval, so its PR waits indefinitely and nobody sees why.
- **No way to repair it from the PR:** the author cannot be edited, and the tag is the one editable field but is ignored whenever the author resolves.

## 5. Proposed fixes

| # | Change | Where | Notes |
|---|---|---|---|
| F1 | **A valid tag that names a known agent wins over the author.** If the tag resolves to an agent different from the author-agent, notify the tag agent. Use the author only when there is no valid tag | `events/review.ts` | Restores the owner's ability to correct routing. The tag is read under the same trusted-head-repo gate as the author, so it adds no new trust. The shared-account case (author unresolvable) is unchanged. **Recommended** |
| F2 | Alternative to F1: notify the **union** of author-agent, tag-agent and committer-agent | `events/review.ts` | Never misses the owner, but the author-agent still gets the false message, with no hint why |
| F3 | When author-agent and tag-agent disagree, add one line to the notice ("PR author is `AgentY-asaf`, tagged for `agent2`") | `format*Notification` | Helps whoever receives it; works with F1 or F2 |
| F4 | AgentMux sets a per-agent git identity at spawn (`GIT_AUTHOR_NAME/EMAIL`, `GIT_COMMITTER_NAME/EMAIL`, e.g. `Agent2 <agent2@agentmux.local>`), and the machine-global `AgentY-asaf` identity is removed | agentmux spawn env; this host's `~/.gitconfig` | Fixes cause 3 for every agent. Removing the global identity is a change to the owner's machine and is not done by this report |
| F5 | Stop mapping unregistered numbered accounts: accept `agent<slot>-<host>` only for hosts registered with the consumer | `agent-mapping.ts` | Fixes cause 2, but reverses the 2026-08-07 host-agnostic rule, which exists so any machine's numbered agents work without registration. Not recommended; F1 makes it unnecessary |

Tests for F1, in `events/review.test.ts` or a new test file: (a) author `AgentY-asaf` + tag `agent2` → notify `agent2` only; (b) author `agentx-workflow[bot]` + no tag → notify `agentx`; (c) author `genericagentx-asaf` + tag `agent2` → notify `agent2` (unchanged); (d) a tag from an untrusted head repo is ignored and the author wins; (e) an invalid tag (`agent_id=../x`) is ignored; (f) tag and author agree → one notification, not two.

## 6. Not verified

- Whether Agent2 received any notice for #448. The consumer's logs are not available to me.
- Why the July PR was opened under `AgentY-asaf` by Agent2's work. A numbered PAT in the environment of an agent that was not slot Y is the likely cause; the PAT era is retired, so no current check can be run.
- Whether other PRs are routed wrongly. Open PRs only, scanned on 2026-10-03: 22 open PRs across `agentmuxai/agentmux`, `agentmuxai/agentmux-cloud`, `a5af/pulse`, `a5af/reagent`, `a5af/shared-infrastructure` and `a5af/dev-tools`, 8 carrying a tag, and **#448 is the only one** whose author-agent differs from its tag. So this is a handover edge case today, not a widespread misroute. Closed PRs and the head-commit path (cause 3) were not scanned.
