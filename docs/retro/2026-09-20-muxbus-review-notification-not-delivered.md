# Retro: MuxBus GitHub review notification never delivered (PR #3448)

**Date:** 2026-09-20
**Author:** Oozp (agent), investigating at the operator's request
**Status:** root cause NOT conclusively confirmed — this is an honest writeup
of what was checked, what was ruled out, and what remains unverifiable from
this agent's own vantage point. Treat §4 as the actionable part.

## 1. What happened

I opened PR #3448 (`agentmuxai/agentmux`, "Oozp@Area54: feat(memory): expose
Global Memory's audit trail...") at approximately 2026-09-20 ~07:50 PDT,
authenticated as the shared `genericagentx-workflow[bot]` GitHub App
identity (this session has no dedicated per-agent App identity
provisioned — see the companion auth-setup work earlier this session).
`reagentx-workflow[bot]` posted a `CHANGES_REQUESTED` review on it at
**2026-09-20T15:09:56Z (08:09:56 PDT)**, confirmed directly via
`GET /repos/agentmuxai/agentmux/pulls/3448/reviews`. Per
`SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md`, that review should
have triggered a MuxBus "jekt" (injected text notification) into this
agent's running conversation, typically within ~5-30 seconds. The operator
had to prompt me about it manually ~15-18 minutes later — no notification
ever arrived.

## 2. What I confirmed

- **The review genuinely landed and is real, actionable content** — two
  findings (a version-downgrade P1, a stale-doc-comment P2), not a delivery
  artifact or a phantom event.
- **The routing/mapping logic, as it exists in a local read-only mirror of
  `agentmux-cloud`, looks correct for this exact case.**
  `agent-mapping.ts`'s `getAgentId()` is *documented and tested* to return
  `undefined` for `genericagentx-workflow[bot]` on purpose (it's the shared
  fallback identity — ambiguous by username alone), which correctly falls
  through to `extractAgentIdFromBody()` in `events/review.ts`
  (`SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`'s username-first/
  tag-fallback priority). My PR body carries
  `<!-- agentmux:agent_id=oozp -->` (verified via the GitHub API that this
  landed correctly in the PR body after I fixed the PR metadata), which
  matches `AGENT_ID_TAG_RE` and `SAFE_AGENT_ID_RE` cleanly. The head repo
  (`agentmuxai`) is in `TRUSTED_REPO_OWNERS`. On paper, this should resolve
  to `targetAgentIds: ["oozp"]` and fire a jekt.
  - **Caveat, load-bearing:** this is a *read-only checkout belonging to a
    different agent* (`manoz-0803a`'s `agentmux-cloud`), last updated
    2026-09-18. I have no way to confirm it matches what's actually
    deployed to the production Lambda right now — `agentmux-cloud` is a
    private repo I still have no write/clone access to myself (separate,
    pre-existing gap — my own `git clone` of it fails auth). If the
    deployed consumer differs from this mirror, everything in the
    paragraph above is moot.
- **This session's own `MUXBUS_TOKEN` (injected into my process env at
  spawn) was expired well before the review landed:** `iat` 07:18:44 PDT,
  `exp` 07:33:44 PDT (a 900-second/15-minute lifetime) — the review posted
  at 08:09:56 PDT, **36 minutes after that specific token expired.**
  - **Important counter-consideration, not dismissed:** `cloud_subscriber.rs`
    has a real proactive-refresh mechanism (confirmed by reading the code —
    a dedicated refresh loop keyed off `refresh_token`, distinct from the
    access token baked into my process's env at spawn time). Environment
    variables are a point-in-time snapshot taken once at process spawn;
    they are **not** continuously updated as the long-running `agentmux-srv`
    sidecar refreshes its own stored credential (`db_muxbus_credentials`)
    in the background. So the fact that *my shell's copy* of the token is
    stale does **not**, by itself, prove the live WS connection was down —
    it's equally consistent with "the sidecar refreshed fine on its own and
    my env var is just a frozen souvenir from spawn time, as expected."
  - I could not resolve this ambiguity. The actual live-connection signal
    (`connected`/`valid`/`expires_at` from the `muxbus.status` RPC command,
    `MuxBusStatusResp` in `muxbus_handlers.rs`) is **only reachable over the
    authenticated WebSocket RPC channel the frontend UI uses**, not a plain
    REST endpoint — I have no tooling from this shell to invoke it.
  - The local `GET /agentmux/discovery` endpoint does show `"oozp"` under
    `wan.local_agents_subscribed` — but that's this sidecar's own
    self-reported local bookkeeping (set via `add_agent()`), not proof the
    upstream authenticated session is currently healthy.
- **Could not check whether the GitHub webhook itself is even configured**
  for this repo (the very first hop, upstream of all agent-mapping logic) —
  `GET /repos/agentmuxai/agentmux/hooks` returned `403 Resource not
  accessible by integration` under the `genericagentx` App identity (no
  `administration`/webhook-read permission granted to that App). This is a
  real gap in what I could verify, not a "checked, found nothing" result.

## 3. Ranked hypotheses

Ranked by likelihood given the above, none confirmed:

1. **Most likely — MuxBus WS connection was genuinely unhealthy at delivery
   time**, for a reason upstream of what I could inspect (a stalled
   reconnect loop, a refresh that silently failed and only backed off
   rather than recovering, or something host/session-specific to this
   particular channel). The expired env-var token is circumstantial, not
   proof, but it's the one concrete anomaly I found, and the discovery
   endpoint's self-reported "subscribed" state is exactly the kind of
   stale-but-still-green status a broken reconnect loop would leave behind.
2. **The deployed `agentmux-cloud` consumer doesn't match the code I read.**
   My only visibility into that repo was a 2-day-stale read-only mirror
   belonging to a different agent session. If a regression shipped there
   since, or if the Lambda hasn't picked up a recent deploy, the
   routing logic I traced through could be entirely moot.
3. **The GitHub webhook isn't actually configured/firing for this repo**,
   consistent with the original spec's §3.4 ("GitHub webhook setup is
   manual... no validation that the webhook is working") never having been
   fully closed out. I could not check this — no permission.
4. **Less likely:** a PR-body-tag parsing edge case. I consider this mostly
   ruled out — I independently re-fetched the PR body via the API after
   editing it and confirmed the tag is present, well-formed, and starts the
   body exactly as `AGENT_ID_TAG_RE` expects.

## 4. Recommended next steps (need access I don't have)

- Have a human (or an agent session with Armory UI access) open Armory →
  Accounts → AgentMux tile and check the connection status directly, or
  trigger `muxbus.status` from the frontend's own RPC channel — this
  single check would directly confirm or rule out hypothesis #1.
- Check `agentmux-cloud`'s actual deployed Lambda version/logs for the
  `consumers/github` function around 2026-09-20T15:09:56Z — confirms or
  rules out #2 and #3 in one step, and is the only way to see whether the
  webhook fired at all. I have no access to do this myself.
- If/when someone with repo-admin GitHub access is available, confirm the
  webhook itself is live via `GET /repos/agentmuxai/agentmux/hooks` (I'm
  blocked on `Resource not accessible by integration` under the current
  shared App identity's permission set).
- Worth separately asking: should `genericagentx`-authenticated sessions
  (i.e. any agent without its own dedicated GitHub App identity) get a
  `webhooks:read`-equivalent permission, or is checking this something only
  a human/dedicated ops identity should ever do? Not my call to make here.

## 5. Related cleanup done alongside this investigation

While reading `bundle_versions.rs` during an unrelated review-comment fix on
PR #3448, found and fixed a stale doc comment
(`bundle_version_list`'s doc, line ~190) that still described
`GlobalMemoryHistory` as "a future... tool" and unbuilt, when PR #3448 itself
wires it in — flagged directly by `reagentx-workflow[bot]`'s review (the
review I never got notified about, ironically found by manually checking the
PR). Fixed in the same PR.
