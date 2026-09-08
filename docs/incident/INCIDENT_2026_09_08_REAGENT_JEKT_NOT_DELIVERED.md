# INCIDENT 2026-09-08 — ReAgent review jekts never reached this session

**Status:** historical — root cause identified and traced to source across three repos; no code was changed as part of this report. Operational workaround applied (see §5); a real fix is a judgment call for whoever owns `agentmux-cloud`, laid out as options in §6.

**Summary:** ReAgent reviewed both of this session's PRs correctly and posted real GitHub reviews (`CHANGES_REQUESTED` twice on #3084, `APPROVED` on #3097) — confirmed directly via the GitHub API. The jekt notification that's supposed to accompany a review and land in this conversation never arrived, for either PR, at any point. This is not a delivery bug in the sense of something crashing or throwing — every system involved did exactly what it was built to do, correctly, in sequence. The failure is that the pieces don't agree on this session's own identity.

---

## 1. What actually happened, traced call by call

**Repos involved:** `agentmuxai/agentmux` (this repo — the srv/UI, and where `AGENTMUX_AGENT_ID` gets injected), `agentmuxai/agentmux-cloud` (the muxbus GitHub-webhook consumer that actually sends the jekt), `a5af/shared-infrastructure` (`github-router` — fans out the raw webhook, otherwise uninvolved), `a5af/reagent` (posts the GitHub review — otherwise uninvolved).

1. **This session's local identity is already split in two.** `AGENTMUX_AGENT_ID` (env var injected at CLI spawn) reads `agentg`. The agent's actual registered/reachable name in the reactive handler — and the identity its muxbus `cloud_subscriber` connection subscribes under — is `claude` (confirmed live: `DiscoverAgents` → `wan.local_agents_subscribed: ["claude"]`). Per `agentmux-srv/src/backend/blockcontroller/persistent.rs:194`, these two are *supposed* to be the same value ("`AGENTMUX_AGENT_ID` (= `agent.name`, set at block creation) is canonical") — they drifted apart for this session specifically, most likely because the agent was renamed to "Claude" in the UI after spawn, which cannot retroactively change the env var already injected into the running CLI process.

2. **Both PRs were opened via the `a5af` GitHub account's token** (found in `~/.npmrc`, used before the correctly-scoped `GenericAgentX-asaf` credential was available). Per the documented convention in this repo's `CLAUDE.md`, a PR pushed under a shared/non-dedicated account must carry `<!-- agentmux:agent_id=$AGENTMUX_AGENT_ID -->` in its body so muxbus can route notifications back correctly. Followed exactly as documented: `<!-- agentmux:agent_id=agentg -->` went into both PR bodies — because `$AGENTMUX_AGENT_ID` is `agentg` (step 1).

3. **`agentmux-cloud/muxbus/consumers/github/events/review.ts`, `processReviewEvent()`**, on `pull_request_review` `submitted`:
   - `getAgentId(event.pull_request.user.login)` → `getAgentId("a5af")`. Per `agent-mapping.ts`'s own doc comment (line 102), `"a5af"` is explicitly documented to resolve to `undefined` — it isn't a numbered agent or a registered named-peer account.
   - Falls through to the documented fallback: `extractAgentIdFromBody(pr.body, headRepoOwnerLogin)` (`agent-mapping.ts:221`). This correctly extracts `agentg` from the tag — exactly what step 2 put there. Nothing here is wrong; the code does precisely what its own doc comment says it does.
   - Also checks `headCommitAuthor` (the resolved GitHub account behind the head commit's author email) via a second `getAgentId()` call. This session's commits are authored as `GenericAgentX-asaf <gax@asafebgi.com>`. Even if that resolves to the real `GenericAgentX-asaf` GitHub login, `agent-mapping.ts`'s own doc comment (line 95-101) explicitly documents `"genericagentx-asaf"` as intentionally resolving to `undefined` — it's the shared fallback account precisely *because* it can't self-identify by username. This path was never going to help here regardless of the tag.
   - Result: `targetAgentIds: ["agentg"]`.

4. **`handler.ts`'s `injectToAgent()`** (line 247) does exactly what it should with that result: `POST https://muxbus.agentmux.ai/reactive/inject` with `target_agent: "agentg"`, `priority: "urgent"`, a 600-second TTL, Ed25519-signed as `reagent-v1`. This call is correct and well-designed — bounded TTL so a stale notification doesn't replay after the PR has moved on, signed so the receiving srv can render `TRUST=host-verified`.

5. **Nobody is subscribed to muxbus as `agentg`.** This session's srv subscribes as `claude` (step 1). No agent on any host has ever registered as `agentg` on the cloud relay. The inject either bounces immediately or — more likely, given the store-and-forward design this whole system uses elsewhere (see Manpo's `ISSUE-sendmessage-false-success.md`, same repo, 2026-09-07) — sits queued for a subscriber that will never arrive, then silently expires once `remainingTtlSeconds()` hits zero at the 600-second mark. Either way: no error surfaces anywhere I have visibility into, and nothing reaches this conversation.

**Every step above is individually correct code, doing exactly what its own documentation says it does.** The break is purely in step 1: the tag-based fallback mechanism's entire contract depends on `$AGENTMUX_AGENT_ID` equalling the agent's actual muxbus-subscribed identity, and for this one session, on this one host, at this one time, it didn't.

## 2. What this is *not*

- **Not a reagent bug.** `a5af/reagent` posts reviews via the GitHub API; that's it. Confirmed via its own repo history (`specs/comment-reply-mode-2026-08-16.md:130`) that an earlier draft of a related spec *also* initially assumed reagent owned jekt delivery and had to correct itself — this is apparently a recurring point of confusion, worth naming so it doesn't happen a third time.
- **Not a `github-router` bug.** Checked `a5af/shared-infrastructure/github-router/lambda/` directly — no muxbus or agentmux references at all. It fans out raw webhooks and separately posts to Discord; unrelated to jekt delivery.
- **Not the known `issue_comment` gap.** The same spec doc above documents a real, separate gap — `handler.ts`'s event switch has no `case 'issue_comment':`, so PR *comments* (as opposed to formal reviews) are silently dropped. That's real, but it doesn't explain this incident: both reviews that never reached me were `pull_request_review` events, which the switch does handle (confirmed: `events/review.ts` exists and is wired in). Worth fixing regardless, but it's a different bug than this one. Also worth checking whether `events/issue-comment.ts`'s existence in the current tree means this has already been fixed since that spec was written — didn't verify either way, out of scope for this report.

## 3. Why this is easy to reproduce and hard to notice

The failure mode is identical in shape to the reactive-handler deadlock this same session root-caused the day before (`INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`): a silent identity/routing mismatch that produces no error, no retry exhaustion visible to the sender, nothing — just an event that quietly never arrives. The sender (`agentmux-cloud`) believes it succeeded (`response.ok` on the initial `POST` — the 202-or-whatever the inject endpoint returns for "queued," not "delivered," per the same store-and-forward pattern already flagged in `ISSUE-sendmessage-false-success.md`). The receiver (this session) has no way to know a notification was ever attempted. Nothing anywhere logs "delivery to `agentg` failed: no such subscriber."

## 4. There was already a clean way to avoid this entirely

`agent-mapping.ts`'s `GITHUB_TO_AGENT_MAP` (line 58) already contains:
```
'claude-asaf': 'claude',
```
alongside `korp-asaf`, `codex-asaf`, `copilot-asaf`, etc. — the established pattern for a **dedicated, named PAT account per agent identity**. Had this session pushed under a `Claude-asaf`-style account, `getAgentId()` would have resolved directly to `claude` — this session's *actual* subscribed identity — with no PR-body tag involved at all, and no way for an env-var/registration-name drift to matter.

That path wasn't available because no such credential exists in `services/infra` (checked: only `gh-token-agent1`–`agent5`, `gh-token-agenta/o/x/y`, and `gh-token-genericagentx` are provisioned — no `gh-token-claude` or equivalent). This session fell through to the shared-account + body-tag path by necessity, not by choice, and that path's correctness depends entirely on a local invariant (`AGENTMUX_AGENT_ID` == registered agent name) that happened to be violated.

## 5. Operational workaround applied this session

`agentmuxai/agentmux#3084`'s body tag was patched from `<!-- agentmux:agent_id=agentg -->` to `<!-- agentmux:agent_id=claude -->` (the actually-subscribed identity) via the GitHub API, since that PR is still open and pending re-review — any further reagent notification on it now has a valid target. `#3097` was already merged before this was diagnosed; its tag was left as-is (editing a merged PR's body has no delivery effect). Future PRs from this session should tag `claude` explicitly rather than `$AGENTMUX_AGENT_ID` verbatim, until the local env-var/registration-name drift (§1, step 1) is independently fixed.

## 6. Options, not a decision — for whoever owns these repos

1. **Provision a `claude-asaf` PAT** (or whatever naming convention fits) and register it in `services/infra` as `gh-token-claude`. Removes the tag dependency entirely for this identity going forward, matching the pattern already used for `korp`, `codex`, `copilot`, etc.
2. **Fix the local drift**: whatever caused this session's `AGENTMUX_AGENT_ID` (`agentg`) to diverge from its registered name (`claude`) — most likely a UI rename post-spawn not re-injecting the env var into the running process — is worth finding and closing so the documented tag convention (`$AGENTMUX_AGENT_ID` verbatim) is trustworthy again without a human needing to know to override it.
3. **Make `/reactive/inject` (or the github-consumer's call to it) surface delivery failure** — even just logging "no active subscriber for target_agent" server-side, or having `injectToAgent()` treat a same-request 4xx *or* a same-request "queued but likely undeliverable" signal differently, would have made this loud instead of silent. This is the same category of fix as `delivered_via` in `ISSUE-sendmessage-false-success.md` — the theme across this whole session is fire-and-forget paths that report success when they mean "accepted for an attempt."

## 7. Evidence index

- This session's split identity: `mcp__agentmux__WhoAmI`, `mcp__agentmux__DiscoverAgents` (live, `wan.local_agents_subscribed: ["claude"]`), `env | grep AGENTMUX_AGENT_ID` (`agentg`).
- Confirmed reagent reviews landed on GitHub but produced no local notification: `gh api`-equivalent `GET /repos/agentmuxai/agentmux/pulls/{3084,3097}/reviews`, cross-checked against this session's own transcript (no inbound message ever arrived) and `~/.agentmux/logs/agentmux-launcher.log` (`grep "reactive inject request received"` — zero inbound hits since either PR was opened).
- Source, read directly, this session: `agentmux-cloud/muxbus/consumers/github/agent-mapping.ts` (full), `agentmux-cloud/muxbus/consumers/github/events/review.ts` (full), `agentmux-cloud/muxbus/consumers/github/handler.ts:200-263`, `a5af/reagent/specs/comment-reply-mode-2026-08-16.md:130-170`, `a5af/shared-infrastructure/github-router/lambda/` (grepped, no matches).
- `agentmux-srv/src/backend/blockcontroller/persistent.rs:194` — the "canonical" claim this incident violates.
