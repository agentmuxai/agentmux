# INCIDENT 2026-09-08 — ReAgent review jekts never reached this session

**Status:** historical — root cause identified and traced to source; no code was changed as part of this report. Operational workaround applied (see §5); a real fix is a judgment call for whoever owns `agentmux-srv`'s reactive registration, laid out as options in §6.

**Correction (2026-09-09, before this PR merged):** reagent's re-review (citing Codex feedback) caught that the first draft of §1 had the causality backwards. Verified directly against source — reagent was right. `agentmux-srv/src/backend/agent_config.rs`'s `build_mcp_config()` states explicitly: "`agent_slug` must be the pre-computed stable role slug... NOT the display name — callers are responsible for passing the right value so renamed agents always advertise the same routing ID." `AGENTMUX_AGENT_ID` (`agentg`) is the system's own documented, intended-stable identity. `claude` — the value actually governing live registration — is this session's current UI display name, and it ended up as the live routing key only because of a separate, deliberate design choice in `agentmux-srv` (§1 step 1a below), not because `agentg` was ever wrong. The original §1 step 1 (below, struck through in spirit, kept for the record rather than deleted) asserted the two were symmetric and had "drifted" from each other with no stated direction; that was imprecise in a way that pointed the recommended fixes (§4, §6) at the wrong target. §1 and §4/§6 are corrected below; §2–3, 5, 7 were unaffected and are unchanged.

**Summary:** ReAgent reviewed both of this session's PRs correctly and posted real GitHub reviews (`CHANGES_REQUESTED` twice on #3084, `APPROVED` on #3097) — confirmed directly via the GitHub API. The jekt notification that's supposed to accompany a review and land in this conversation never arrived, for either PR, at any point. This is not a delivery bug in the sense of something crashing or throwing — every system involved did exactly what it was built to do, correctly, in sequence. The failure is that the pieces don't agree on this session's own identity.

---

## 1. What actually happened, traced call by call

**Systems involved:** `agentmuxai/agentmux` (this repo — the srv/UI, and where `AGENTMUX_AGENT_ID` gets injected), the cloud relay's GitHub-webhook consumer (a private service that actually sends the jekt), a webhook fan-out service (otherwise uninvolved), and ReAgent (posts the GitHub review — otherwise uninvolved).

1. **This session's local identity is split in two, and the system's own code is explicit about which half is authoritative.** `AGENTMUX_AGENT_ID` (env var injected at CLI spawn) reads `agentg`. Two independent places in `agentmux-srv` document this as the *stable, intended* routing identity: `agent_config.rs`'s `build_mcp_config()` ("must be the pre-computed stable role slug... NOT the display name — callers are responsible for passing the right value so renamed agents always advertise the same routing ID") and `server/agent_handlers/input.rs:709-713` ("PR bodies embed `$AGENTMUX_AGENT_ID` (same value) so the cloud injection key and the poll key are always consistent").

   1a. **What actually governs live registration is `input.rs`'s Register-tail, and it deliberately does not use that stable value.** On every ordinary UI-driven turn (`TurnRegistration::Register`, `input.rs:706-721`), the reactive handler is re-registered under `block.meta["agentName"]` — the *current* display name (`agent_open.rs:536`: `meta.insert("agentName", &agent.name)`), not `AGENTMUX_AGENT_ID`. This is not an oversight; the surrounding comment (`input.rs:722-734`) explains it was chosen deliberately, to fix a *different*, earlier bug (#2695/#2697): a stale spawn-time-captured identity was causing the recipient-identity check to falsely reject an agent's own correctly-addressed jekts after a rename. That fix's side effect is this incident's proximate cause: `register_agent` replaces any existing mapping (confirmed reading `Handler::register_agent_with_nonce`), so the very next ordinary turn after any post-spawn rename silently overwrites the correct, `agentg`-keyed spawn-time registration with a `claude`-keyed one — and nothing detects or warns that the two documented "same value" fields have diverged. Confirmed live: `DiscoverAgents` → `wan.local_agents_subscribed: ["claude"]`, not `agentg`.

   In short: `agentg` was never wrong. `claude` is what a UI rename plus the very next keystroke does to the live registration, and the PR-body-tag convention (step 2) has no way to know that happened.

2. **Both PRs were opened under a shared GitHub account** that cannot identify an individual agent by its username. Per the documented convention in this repo's `CLAUDE.md`, a PR pushed under a shared/non-dedicated account must carry `<!-- agentmux:agent_id=$AGENTMUX_AGENT_ID -->` in its body so muxbus can route notifications back correctly. Followed exactly as documented: `<!-- agentmux:agent_id=agentg -->` went into both PR bodies — because `$AGENTMUX_AGENT_ID` is `agentg` (step 1).

3. **The cloud relay's GitHub consumer**, on `pull_request_review` `submitted`, could not map the PR author (a shared account) or the head-commit author (also a shared account) to an agent, so it fell back to the PR-body tag and correctly extracted `agentg` — exactly what step 2 put there. Nothing here is wrong; the consumer does precisely what it is documented to do.

4. **The consumer then injected the notification** for `target_agent: "agentg"` through the relay, with a bounded TTL (so a stale notification doesn't replay after the PR has moved on) and signed so the receiving srv can verify it.

5. **Nobody is subscribed to muxbus as `agentg`.** This session's srv subscribes as `claude` (step 1). No agent on any host has ever registered as `agentg` on the cloud relay. The inject either bounces immediately or — more likely, given the store-and-forward design this whole system uses elsewhere (see Manpo's `ISSUE-sendmessage-false-success.md`, same repo, 2026-09-07) — sits queued for a subscriber that will never arrive, then silently expires with its TTL. Either way: no error surfaces anywhere I have visibility into, and nothing reaches this conversation.

**Every step above is individually correct code, doing exactly what its own documentation says it does.** The break is purely in step 1: the tag-based fallback mechanism's entire contract depends on `$AGENTMUX_AGENT_ID` equalling the agent's actual muxbus-subscribed identity, and for this one session, on this one host, at this one time, it didn't.

## 2. What this is *not*

- **Not a reagent bug.** ReAgent posts reviews via the GitHub API; that's it. It does not own jekt delivery — a recurring point of confusion, worth naming so it doesn't happen again.
- **Not a webhook fan-out bug.** The fan-out service is unrelated to jekt delivery.
- **Not the known PR-comment gap.** PR *comments* (as opposed to formal reviews) were at one point not routed as jekts. That doesn't explain this incident: both reviews that never reached me were `pull_request_review` events, which are routed.

## 3. Why this is easy to reproduce and hard to notice

The failure mode is identical in shape to the reactive-handler deadlock this same session root-caused the day before (`INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`): a silent identity/routing mismatch that produces no error, no retry exhaustion visible to the sender, nothing — just an event that quietly never arrives. The sender (the cloud relay) believes it succeeded (`response.ok` on the initial `POST` — the 202-or-whatever the inject endpoint returns for "queued," not "delivered," per the same store-and-forward pattern already flagged in `ISSUE-sendmessage-false-success.md`). The receiver (this session) has no way to know a notification was ever attempted. Nothing anywhere logs "delivery to `agentg` failed: no such subscriber."

## 4. Why the "obvious" fix (provision a dedicated per-agent account) is wrong

An earlier draft of this report suggested provisioning a dedicated per-agent GitHub account for `claude`, the established pattern for agents with their own account. reagent's re-review correctly rejected this: it would hard-code `claude` — a renameable UI display name — as if it were a stable identity. The very next rename would silently break it again, exactly the way `agentg` broke this time, just with an extra layer of GitHub-account plumbing built on top of the same bug. A dedicated account is the right pattern for a genuinely stable identity; it is not a fix for a value that isn't stable in the first place.

The real fix has to live where the divergence is created: §1a, `input.rs`'s Register-tail. See §6.

## 5. Operational workaround applied this session

`agentmuxai/agentmux#3084`'s body tag was patched from `<!-- agentmux:agent_id=agentg -->` to `<!-- agentmux:agent_id=claude -->` (the actually-subscribed identity) via the GitHub API, since that PR is still open and pending re-review — any further reagent notification on it now has a valid target. `#3097` was already merged before this was diagnosed; its tag was left as-is (editing a merged PR's body has no delivery effect). Future PRs from this session should tag `claude` explicitly rather than `$AGENTMUX_AGENT_ID` verbatim, until the local env-var/registration-name drift (§1, step 1) is independently fixed.

## 6. Options, not a decision — for whoever owns `agentmux-srv`'s reactive registration

1. **Register under `AGENTMUX_AGENT_ID`, not `block.meta["agentName"]`, in `input.rs`'s Register-tail** — the change that would have prevented this. Doing so re-opens the #2695/#2697 bug that motivated the current behavior (a stale spawn-time identity falsely rejecting the agent's own jekts after a rename) unless that's fixed a different way at the same time — e.g. resolve the recipient-identity check against the live env-injected slug instead of the block's captured-at-spawn value, so both problems close together instead of trading one for the other again.
2. **Detect the divergence instead of silently preferring one side.** When `agentName` (current display name) and `AGENTMUX_AGENT_ID` (stable slug) disagree at registration time, log it, or surface it in the UI next to the rename control — a human renaming an agent has no way to know it just silently changed which routing key GitHub notifications need.
3. **Make `/reactive/inject` (or the GitHub consumer's call to it) surface delivery failure** — even just logging "no active subscriber for target_agent" server-side, or having the caller treat a same-request 4xx *or* a same-request "queued but likely undeliverable" signal differently, would have made this loud instead of silent regardless of which side of #1/#2 ships. Same category of fix as `delivered_via` in `ISSUE-sendmessage-false-success.md` — the theme across this whole session is fire-and-forget paths that report success when they mean "accepted for an attempt."

## 7. Evidence index

- This session's split identity: `mcp__agentmux__WhoAmI`, `mcp__agentmux__DiscoverAgents` (live, `wan.local_agents_subscribed: ["claude"]`), `env | grep AGENTMUX_AGENT_ID` (`agentg`).
- Confirmed reagent reviews landed on GitHub but produced no local notification: `gh api`-equivalent `GET /repos/agentmuxai/agentmux/pulls/{3084,3097}/reviews`, cross-checked against this session's own transcript (no inbound message ever arrived) and `~/.agentmux/logs/agentmux-launcher.log` (`grep "reactive inject request received"` — zero inbound hits since either PR was opened).
- Source, read directly, this session: the cloud relay's GitHub consumer (private).
- `agentmux-srv/src/backend/blockcontroller/persistent.rs:194` — the "canonical" claim this incident violates.
- Correction evidence (2026-09-09): `agentmux-srv/src/backend/agent_config.rs`'s `build_mcp_config()` doc comment (the "stable role slug... NOT the display name" line), `agentmux-srv/src/server/agent_handlers/input.rs:706-734` (the Register-tail, its `agentName`-as-key comment, and the #2695/#2697 rationale for it), `agentmux-srv/src/server/app_api/agent_open.rs:536` (`agentName` meta set from `agent.name`).
