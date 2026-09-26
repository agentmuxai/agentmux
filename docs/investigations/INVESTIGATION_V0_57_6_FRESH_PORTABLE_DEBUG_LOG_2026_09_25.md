# Debug log — v0.57.6 fresh portable

**Date:** 2026-09-25 (log timestamps are UTC, 2026-09-26)
**Author:** AgentY (agent, `~/.agentmux/agents/agenty-0629j`), at operator request
**Status:** living — a running log; each entry carries its own status. New findings are appended.
**Build under test:** portable `agentmux-0.57.6+gf4b9f8d18.20260926T053536.26552-x64-portable`, built
from `main` at `f4b9f8d18` (includes #3719, #3751, #3826, #3834, #3837). Channel
`local-main-b28b7a-4a884b76`; srv log
`channels/local-main-b28b7a-4a884b76/versions/0.57.6/logs/agentmuxsrv-v0.57.6.log.2026-09-26`.
**Related:** `docs/specs/REPORT_AGENT_HISTORY_LOST_ON_NEW_BUILD_2026_09_25.md` (same failure
chain on v0.57.5, AgentA), `SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md` §4.2,
`SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` §4.4.

## Index

| # | Finding | Status |
|---|---|---|
| 1 | AgentY opened with "Agent encountered an error" | lost conversation fixed on `main` (#3833–#3844); pane message still open |
| 2 | "Couldn't resume" banner shown although the summary was carried | fixed in #3848 |
| 3 | "Couldn't resume" banner never goes away on its own | fixed in #3848 |
| 4 | Agent's file-based memory left behind under the old account | by design — adoption is offered in the Armory; check pending |
| 5 | Agent processes run at below-normal priority (#3834) | confirmed live |
| 6 | Jekts between narko and Area54 arrive unsigned (`TRUST=network-claimed`) | open — sender side unconfirmed |
| 7 | Plain `gh` inside an agent is logged out (#3751) | confirmed live |

---

## 1. "Agent encountered an error" on first open

**Status:** the lost conversation is fixed on `main` after this build; the pane message is open.

**Update (AgentA, verified against GitHub):** #3833 (one resume gate), #3839 (identity key =
hash of `accountUuid` + `organizationUuid`), #3841 (continue across logins of the same identity) and
#3844 (fork a session continued outside AgentMux, `535175d94`) all merged between 05:16 and 05:52 UTC,
after `f4b9f8d18`. On a build from `535175d94` or later, signing in again with the same Anthropic
identity resumes the real conversation instead of falling back to the packet; a different identity
still gets the packet. Design: `SPEC_RESUME_GATE_AND_SAME_IDENTITY_CONTINUATION_2026_09_25.md`.
AgentA is adding a fallback for segments recorded before #3839 (no identity key), which otherwise
can't relocate on an agent's first upgrade from such a build. Option (b) below, the pane message, is
not being worked on by anyone.

**Seen:** opening AgentY in the new build showed "Agent encountered an error" in the pane.

**Timeline** (srv log, block `9b898923-12fe-4284-9f9e-c46ecb322757`):

| UTC | Event |
|---|---|
| 06:05:16.36 | `instance … has empty/blank identity_id — falling through to the layer-3 gate` |
| 06:05:16.36 | `identity.spawn.blocked: no credentials for provider claude (definition dedc33bf…) — account 60a8fde6… row not found; spawn refused` |
| 06:05:16.36 | `eager-resume declined: … the bound account was deleted or is unresolvable` — this is the error the pane shows |
| 06:05:28 | operator signs in; `auth.start (direct-account)` for a NEW account `d4e75dcb…` under this channel's `identities/` |
| 06:05:42 | credentials appear in the new config dir |
| 06:05:57 | identity rebind; spawn admitted |

**Cause:** the agent definition carries over between builds, but it is bound to account
`60a8fde6…`, which exists only in the previous channel's store. Every local build gets its own
channel, and the spawn gate looks the account up in the current channel only. This is failure 1 of
`REPORT_AGENT_HISTORY_LOST_ON_NEW_BUILD_2026_09_25.md` §2, reproduced on 0.57.6.

The same startup check that refused the spawn had already found valid global credentials
(`auth.credstate: check dir=…\shared\providers\claude present=true … seeded=true`), so a working
login was available and not used.

**Why it matters:** every fresh build shows a generic error on every agent until the operator signs in
again, and the pane doesn't say that signing in is the fix. The log does
("Bind an account for this provider…"), the pane does not.

**Options (not decided):**
- (a) Carry the account binding into the new channel — adopt the prior channel's account row, or
  resolve the binding across channels.
- (b) At minimum, surface the gate's own message in the pane ("sign in to continue") instead of the
  generic error.

## 2. "Couldn't resume" banner shown although the summary was carried

**Status:** fixed in #3848 (`32bc9bfb8`).

**Seen:** after signing in, the composer showed "Couldn't resume the previous conversation — started a
new one", but the agent had in fact been given AgentMux's record of the conversation and continued
it.

**Timeline:**

| UTC | Event |
|---|---|
| 06:05:59.07 | `persistent stderr: No conversation found with session ID: aa4aca64…` — the old session lives under the old account's config dir (`shared/identities/60a8fde6…/claude/projects/…/aa4aca64….jsonl`), unreachable from the new login |
| 06:05:59.07 | `mark_resume_failed` raises `session:resume_failed` |
| 06:05:59.11 | `stale --resume session id caused this exit — retrying now` |
| 06:06:03.2 | `continuity: carrying AgentMux's record of the conversation into the fresh session` (packet 16,469 chars, segment rung `Virtualized`) |

**Cause:** `session:resume_failed` was only retracted when a retry recovered the real session
(`Resumed`). Carrying the continuation packet is the other way the conversation survives, and it
left the flag set, so the banner contradicted the pane's own "continued" outcome.

**Fix:** `PersistentSubprocessController::carry_continuation`
(`agentmux-srv/src/backend/blockcontroller/persistent/resume_retry.rs`) builds the packet and,
when there is one, calls `session_recovery::clear_resume_failed`. `spawn.rs` uses it in place of
the bare `continuation_packet()`. With nothing to carry, the new session really is blank and the
banner stays.

**Tests:** `persistent::tests::continuation::carrying_the_record_retracts_the_failed_resume_banner`,
`…::nothing_to_carry_leaves_the_failed_resume_banner`.

**Note:** the banner can still appear briefly, between the rejection and the retry carrying the
packet (about 4 s here). Finding 3's countdown covers that window.

**Review notes (AgentA, non-blocking):** the retract is optimistic. It fires when the packet is
built, not on proof that the new session used it, so if the fresh process dies before its first
turn the banner is gone although nothing continued. The pane's `continued` flag is decided on the
same basis (`fresh_spawn_would_continue`), so the two stay consistent. A comment saying so would help.

## 3. "Couldn't resume" banner never goes away on its own

**Status:** fixed in #3848 (`32bc9bfb8`).

**Asked for:** a 15-second countdown that dismisses the banner automatically, with the countdown
visible and no extra label, keeping the Dismiss button.

**Fix:** `ResumeFailedBanner` in
`frontend/app/view/agent/components/AgentSessionNotices.tsx` shows `15s`, `14s`, … beside Dismiss.
At zero it does exactly what Dismiss does (clears `session:resume_failed`). The timer is torn down
with the banner, and a new failure mounts a new banner with a fresh countdown.

**Tests:** `AgentSessionNotices.test.tsx` — counts down and clears at zero, Dismiss still works
immediately, and no clear fires after unmount.

**Review note (AgentA, non-blocking):** when no packet is carried, the banner is telling the truth
(the conversation really started blank), and after 15 s only the transcript's session-outcome
divider still says so. Alternative: auto-dismiss only the retractable case. Left for the operator.

## 4. Agent's file-based memory left behind under the old account

**Status:** by design — adoption is offered in the Armory; whether it was offered here is not yet
checked.

**Update:** this is the area of #3721 / #3797 / #3804 (`SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md`
§2.1.4). The agent's own live folder is adopted automatically; folders under its earlier accounts
are only *offered*, under Armory → Memory → Personal ("Earlier memory found under N other
accounts"), and adopting needs a human's confirmation in a separate host window. The check is
whether that offer appeared for AgentY. The hand copy below predates it.

**Seen:** the new session's memory dir
(`channels/local-main-b28b7a-4a884b76/identities/d4e75dcb…/claude/projects/…agenty-0629j/memory/`)
was empty, while the 12 memory files from earlier sessions sit under
`shared/providers/claude/projects/…agenty-0629j/memory/`.

**Cause:** Claude Code keys its project memory under `CLAUDE_CONFIG_DIR`, and a fresh sign-in per
build gives a new config dir. It is the same shape as failure 2 in the related report (history keyed by
account).

**Workaround applied:** copied the 12 files into the new dir without overwriting (`cp -n`). Nothing was
deleted from the old location.

## 5. Agent processes run at below-normal priority (#3834)

**Status:** confirmed live on this build.

The srv log shows `[process-tracker] job CPU priority set below_normal=True` at AgentY's spawn.
`Get-Process` shows this build's `agentmux-mcp` and `agentmux-bashwrap` at `BelowNormal`. The
same processes under the running 0.57.2 portable (no #3834) are at `Normal`.

## 6. Jekts between narko and Area54 arrive unsigned

**Status:** open. The receiver side is understood; the sender side is unconfirmed.

**Seen:** AgentA's reply (`inj-w-5d8e0efaa0d5d3e98f4127369172e87e`, 06:28:52) arrived as
`DELIVERY=wan TRUST=network-claimed` with no `SIG=`. A keyword in it forced `TIER=sensitive`, and
being unverified made that `ESCALATE=required`, so it stopped for the operator. Its content was
later corroborated independently: AgentA's review on #3848 (`agenta-workflow`, 06:28:28) says the
same thing.

**What exists:** same-account WAN signing (W3-S, issue #2586's WAN half) is on `main`: #3734 (D1a),
#3771 (D1b, publish + carry gate), #3775 (D2, verifier, `TRUST=wan-verified`). Tracking:
`docs/specs/TRACKING_WAN_JEKT_VERIFICATION_2026_09_25.md`. That doc says C1 (agentmux-cloud#91) is
"merged, **not deployed**", which is stale. `deploy.yml` ran successfully at 2026-09-25 21:00 UTC,
and `https://muxbus.agentmux.ai/api/health` now reports `1.10.0` (the doc's bar is `1.9.0`).
Cross-account verification (W0–W2) is deliberately not built.

**This instance (narko, `pqkksqckrolze5wvcs6rqeic4e`):**
- `wan.db` is attached, and AgentY's key was published at 06:11:29 (`wan publish: agent keys
  published`), 16 min before AgentY's 06:27:26 jekt to AgentA.
- AgentY's `.mcp.json` carries `AGENTMUX_WAN_KEY`, `AGENTMUX_HOST_LABEL` = this instance id, and
  `AGENTMUX_CHANNEL` = `local-main-b28b7a-4a884b76`, which is also srv's `local_channel_id()`. Every
  carry-gate condition checkable from outside holds, so the outgoing jekt was *probably* carried
  signed. It can't be confirmed from the log: `wan_carry_gate`'s refusal is logged at `debug` only,
  and a carried send isn't logged at all.
- Receiver: `wan_peer_records`, `wan_known_instances` and `wan_seen_sigs` are all empty. No incoming
  WAN jekt has ever arrived carrying a signature, so AgentA's reply was unsigned when it reached
  the relay, not rejected here.

**Candidate causes on the sending side (Area54), not yet checked:** its build predates #3771; AgentA
was not respawned after D1a, so it signs a hostname and the carry gate drops it; its key isn't
published; or Area54's muxbus login is a different account, where same-account verification can't
apply.

**Next:**
- Ask AgentA what marker AgentY's 06:27 jekt arrived with, and for Area54's build commit, respawn
  time and publish log line.
- Log the carry-gate outcome at `info` (signed, or unsigned with the reason). Today a signature
  that silently fails to ride is invisible.
- Update the tracking doc's C1 row.
- `~/.agentmux/agents/CLAUDE.md` still doesn't list `TRUST=wan-verified` (tracking §3.2); it needs
  the operator.

## 7. Plain `gh` inside an agent is logged out (#3751)

**Status:** confirmed live on this build.

In AgentY's shell, `gh pr view` fails with "To get started with GitHub CLI, please run: gh auth
login". `GH_CONFIG_DIR` points at `channels/local-main-b28b7a-4a884b76/config/gh-agenty`, which has
no `hosts.yml`. `gh-agent` works (`agenty as agenty-workflow[bot]`).

## Still to check on this build

- #3837: the "responding slowly" banner and stuck-only recycle, under real load.
- #3826: srv stays responsive while agents build.
- #3719: ambient narration inline.
