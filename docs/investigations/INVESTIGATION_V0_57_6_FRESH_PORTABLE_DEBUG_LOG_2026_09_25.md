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
| 6 | Jekts between narko and Area54 arrive unsigned (`TRUST=network-claimed`) | open — receiver's key lookup fails; diagnostics in #3858 |
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

**Status:** open. Signing and carrying work on both sides. The receiver's directory fetch fails
(`wan_key_unavailable`), cause not yet known. Logging that names it is in #3858.

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
- Receiver: `wan_peer_records`, `wan_known_instances` and `wan_seen_sigs` are all empty.
  (**Corrected below:** this doesn't mean nothing signed arrived. The verifier caches a peer record
  only after a successful fetch.)

**Area54's side (AgentA, 07:13):** AgentY's jekts arrived there as `TRUST=network-claimed`, no
`SIG=`, too. Area54 runs v0.57.5 @ `089af6ebe` (includes #3734/#3771/#3775). AgentA's `.mcp.json` has
`AGENTMUX_WAN_KEY` and `AGENTMUX_HOST_LABEL` = its instance id `m5mfalbsxiwm4oltsas4kjwxli`, and it
published at 04:37:08, after PKCE login. Both installs are logged in to muxbus as the same account.

**Ruled out:**
- *Different accounts:* narko's srv log also shows `muxbus: PKCE login succeeded`, same email.
- *Key rotated under a running agent:* on narko, the public key derived from AgentY's
  `AGENTMUX_WAN_KEY` equals `wan.db`'s published `agenty` key.
- *Not carried:* every jekt between the two has an `inj-w-<hash>` id. The cloud assigns that
  (`wan-keys.ts` `wanIdempotentInjectionId`) only to a row stored with a *valid* carried tuple
  from an account-bound sender. So both sides signed, and the relay carried and stored the
  signatures.

**Where it breaks — the receiver's directory fetch.** The injection audit
(`GET /agentmux/reactive/audit`, in memory) records the verdict for every WAN jekt:

| UTC | From → to | `wan.reason` |
|---|---|---|
| 06:28:52 | agenta → agenty | `wan_key_unavailable` |
| 07:13:07 | agenta → agenty | `wan_key_unavailable` |
| 13:32–13:39 (5) | opaz → agent2 | `wan_key_unavailable` |

`wan_key_unavailable` comes after `sender_same_account == true` and the envelope check pass: the
`GET /agents/<agent>/wan-key` lookup failed twice. That covers five different causes that the code
didn't tell apart: a non-404 HTTP status (the route 403s a token with no account, 429s over 120/min,
400s a malformed query), a transport error or the 2 s timeout, a 200 that doesn't parse as a
`WanKeyRecord`, or the local fetch budget running out. Unauthenticated, the route answers 401 in
about 0.27 s, so it is deployed and reachable. Its response shape matches `WanKeyRecord`.

**Diagnostics (#3858):** the verdict now carries a `detail` naming which of these it was, in the
audit and in an `info` line per WAN jekt (`wan verify: outcome`). The sender's "queued for WAN
delivery" line says `signed`, `unsigned_reason` and `cloud_kept_signature`.

**Next:**
- After #3858 is in a running build, send one signed jekt each way and read `detail`.
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
