# SPEC: GitHub review-notification agent detection — username-first, tag as fallback

**Date:** 2026-08-07
**Status:** Implemented
**Author:** Agent3
**Repos touched:** `agentmux-cloud` (implementation), `agentmux` (this doc + agent-facing policy)
**Supersedes:** §3.2 and §5 of `SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md` — that doc's "Priority: extracted ID > static mapping > regex pattern" (tag checked first, username as fallback) is no longer accurate; see §1 below for why the order flipped.

---

## 1. What changed and why

The GitHub PR-review webhook consumer (`agentmux-cloud/muxbus/consumers/github/`)
decides which agent to notify when a review lands on a PR. Previously it
checked the `<!-- agentmux:agent_id=... -->` PR-body tag **first**, falling
back to the PR author's GitHub username only when no tag was present.

That's backwards. A **standard agent identity** — a numbered PAT/App
account (`Agent3-<host>`, `agentx-workflow[bot]`) or a registered named peer
account (`korp-asaf`) — is unambiguous on its own; the username alone
already says exactly which agent opened the PR. The tag only earns its keep
for agents that *don't* have one of these — which in practice means an
agent pushing PRs under the **shared `GenericAgentX-<host>` account**
(`agentmux` repo's `CLAUDE.md`, "Which GitHub account am I acting as?"
section — the fallback identity used when an agent has no dedicated PAT
registered). That shared account's username can't disambiguate which of
several possible agents actually authored a given PR; only the tag can.

**New priority, in `consumers/github/events/review.ts`'s `processReviewEvent`:**

1. Resolve the PR author's GitHub username via `getAgentId()`
   (`agent-mapping.ts`). If it resolves — a standard agent — notify that
   agent. **The tag is not even read in this case.**
2. Only when the username does **not** resolve to a known agent, extract
   and validate the `<!-- agentmux:agent_id=... -->` tag from the PR body
   (still gated on a trusted HEAD repo owner — unchanged, see
   `extractAgentIdFromBody`'s existing fork-abuse protection) and notify
   that agent instead.

The head-commit-author check (a separate, independent signal — notifies a
second agent if it contributed commits but isn't the PR author) is
unchanged by this: it was never tag-aware to begin with, and still isn't.

## 2. Host-agnostic numbered-agent detection

`agent-mapping.ts`'s numbered-PAT pattern was hardcoded to a single host
suffix: `/^agent([xya-g]|\d)-asaf$/`. Any other host running the same
numbered-agent convention (e.g. `Agent3-XYUO`, `Agent1-12345` — different
machines/humans using their own instance of the numbered-agent scheme)
fell through unrecognized, forcing them onto the tag-fallback path even
though their username was just as self-describing as `-asaf`'s.

Widened to `/^agent([xya-g]|\d)-[a-z0-9]+$/` — any host suffix is
accepted; the numbered slot (`agent3`, `agentx`, …) is extracted and the
host discarded, since the resulting agent ID is host-agnostic by design
(see §3). Named peer accounts (`korp-asaf`, `loap-asaf`, …) are
unaffected — they remain exact-match entries in the static
`GITHUB_TO_AGENT_MAP`, not a generalized pattern, so an arbitrary
`{anything}-asaf` account still can't self-admit as a known agent.

## 3. One agent, multiple channels/hosts — already handled downstream

An agent ID like `agent3` can be actively running on more than one
AgentMux channel at once — even across multiple physical hosts belonging
to the same person. This detection layer doesn't need to know or care:
delivery is keyed purely on the resolved agent ID string. Every live
`cloud_subscriber` instance polling under that ID (`GET
/reactive/pending/agent3`) competes for the same pending queue, and the
atomic-claim `/reactive/ack` endpoint (`muxbus/server/src/index.ts`, see
its "atomic injection claim prevents cross-channel duplicate delivery"
history) ensures exactly one of them wins — no duplicate delivery, no new
code needed here. This spec's only job is producing the correct target ID
string; fan-out/dedup across channels was already solved by prior work.

This assumes single-tenant semantics: the numbered slot (`agent3`) is
scoped to one person's/org's fleet, not globally unique across unrelated
AgentMux users. If cross-tenant collisions ever become a real scenario
(two unrelated people both running an "Agent3"), the credential-binding
check already in `server/src/index.ts` (`checkAgentBinding`) is the
natural place to add tenant scoping — out of scope here since this
environment is single-tenant today.

## 4. Agent-facing policy (this repo)

Nothing in the agent-side workflow changes in the common case: continue
including the `<!-- agentmux:agent_id=... -->` tag on every PR per
`CLAUDE.md`'s existing instruction — it's harmless (and ignored) when
you're pushing as your own standard identity, and it's what makes
notifications work at all when you're not.

**When the tag is load-bearing, not just decoration:** if `gh-agent.sh`
had to fall back to the shared `gh-token-genericagentx` key (no dedicated
PAT registered for you — see `CLAUDE.md`'s "Which GitHub account am I
acting as?"), the tag is the *only* signal the review-notification
pipeline has to route to you. Omitting it in that specific case means
reviews on that PR are silently never delivered — not a new failure mode
(this was already true before this change), but worth stating plainly now
that the priority order makes it the exclusive path for generic-account
PRs rather than one of two.

## 5. Testing

`agentmux-cloud/muxbus/consumers/github/agent-mapping.test.ts` and
`events/review.test.ts` (new — this package had no test coverage before
this change) cover: host-agnostic numbered-pattern resolution across
multiple hosts, named-peer static-map matching, rejection of the ReAgent
reviewer bot and the generic fallback account, the full username-first/
tag-fallback priority (including "tag present but ignored because the
author already resolved," and "tag rejected — untrusted fork owner" /
"tag rejected — malformed value"), and the committer-notification path
(including dedup when committer and author resolve to the same agent).

## 6. Files changed

| Repo | File | Change |
|---|---|---|
| `agentmux-cloud` | `muxbus/consumers/github/agent-mapping.ts` | Host-agnostic numbered-PAT pattern (§2) |
| `agentmux-cloud` | `muxbus/consumers/github/events/review.ts` | Reversed priority: username first, tag fallback (§1) |
| `agentmux-cloud` | `muxbus/consumers/github/agent-mapping.test.ts` | New |
| `agentmux-cloud` | `muxbus/consumers/github/events/review.test.ts` | New |
| `agentmux-cloud` | `muxbus/consumers/github/package.json` | Added `vitest` + `test` script (package had none) |
| `agentmux` | `docs/specs/SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` | This doc |
| `agentmux` | `CLAUDE.md` | §4 policy clarification |
