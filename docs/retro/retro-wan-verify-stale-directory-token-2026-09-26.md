# Retro: WAN jekt verification fell back to `network-claimed` after 15 minutes — 2026-09-26

**Status:** retro

## Summary

W3-S same-account WAN verification (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`,
tracking in `TRACKING_WAN_JEKT_VERIFICATION_2026_09_25.md`) worked in one
direction and not the other during the first two-machine check. narko → charlie
arrived `TRUST=wan-verified`; charlie → narko arrived `TRUST=network-claimed`
every time.

The sender side was fine on both machines. The receiver's key-directory lookup
was authenticated with the cloud connection's shared token, which
`cloud_subscriber` loads **once, at connect**. A desktop (PKCE) access token
lives **15 minutes** (`muxbus-cognito.ts`, `accessTokenValidity`); a connection
lives up to **2 hours**. From minute 15 of every connection, every
`GET /agents/:id/wan-key` was a 401, which `wan_verify::get_json` reads as
"couldn't check" → `wan_key_unavailable` → delivered as `network-claimed`, with
nothing logged at any level above debug.

So verification only worked in the first 15 minutes of each 2-hour connection.
charlie verified narko's message because its connection was 5.5 minutes old;
narko's was 80 minutes old.

## Impact

- Every same-account WAN jekt received more than 15 minutes into a connection
  arrived `network-claimed` instead of `wan-verified`. That is the pre-W3-S
  behaviour, so nothing got less safe, but the feature was effectively off
  most of the time.
- Messages that contained a sensitive keyword were stopped for operator
  approval (`ESCALATE=required`) when a verified sender would have been tagged
  `ESCALATE=none`. Two of Opaz's messages during the check stopped this way.
- No message was lost or wrongly marked verified.

## Timeline (UTC, 2026-09-26)

| Time | Event |
|---|---|
| 2026-09-25 21:00 | agentmux-cloud `main` (4181d6c, #91 C1 + #92 leases) deployed; `/api/health` 1.10.0. |
| 06:11:29 | narko's cloud connection opens with a fresh desktop token. |
| 06:28:52, 07:13:07 | agenta → agenty arrive `wan_key_unavailable` (17 and 62 min into the connection). Unnoticed. |
| 07:08 | Round 1 with Opaz: both directions `network-claimed`. Explained correctly by charlie running 0.57.4, which predates the verifier and carry gate. |
| 12:11:47 | narko's connection reconnects (2-hour cycle). |
| 13:26:33 | charlie, upgraded to 0.57.6, connects. |
| 13:27:26 | narko → Opaz round 2 sent; held by the relay until Opaz is respawned (13:30:56); delivered 13:32:07 as `wan-verified` (charlie's connection 5.5 min old). |
| 13:32:08 – 13:35 | Four Opaz → narko messages arrive `network-claimed` (narko's connection 80+ min old). |
| ~13:45 | narko's `/agentmux/reactive/audit` shows `wan.reason = wan_key_unavailable` for all four. |
| ~13:55 | Cloud route and auth read: 401/403/429/5xx/timeout all map to "unavailable"; route latency ~0.2 s rules out the 2 s timeout; Lambda logs carry no path/status. Token lifetime (15 min) found; the table of connection ages matches every observed outcome. |

## How we found it

1. Confirmed the cloud was deployed at `main` and the publisher had published
   narko's keys (srv log `wan publish: agent keys published`).
2. Round 1 failed both ways; charlie's version (0.57.4) explained it fully, so
   nothing was changed.
3. Round 2: narko → charlie verified, charlie → narko did not. Opaz checked
   every carry-gate condition on charlie from the outside: config, `wan.db`,
   publish time, relay queue lines. All held.
4. The receiver-side reason was already recorded, just not logged: the
   in-memory audit log (`GET /agentmux/reactive/audit`) carries a `wan` block
   per delivery. It said `wan_key_unavailable`, not `wan_key_not_found`, which
   meant the directory answered with something other than 200 or 404.
5. Reading `wan_verify::get_json` → `Directory { token }` → `sync_agent_reactive(token)`
   → `connect_and_run(&token)` → `load_valid_token` once per connection,
   against `accessTokenValidity: minutes(15)`, gave the mechanism. Every
   observed outcome fits connection age > 15 min ⇒ unavailable.

## Root cause

`sync_agent_reactive` already preferred a per-agent credential for the mail
fetch and fell back to the connection's token only on rejection, so mail kept
working and the stale token was never exercised on the hot path. The D2
verifier (#3775) reused the same `token` parameter for the directory lookup
without that treatment. `wan_publish` and `relay::relay_token` both call
`load_valid_token` per use; the verifier was the odd one out.

## Why it wasn't caught

- **Tests authenticate with a constant.** The subscriber tests call
  `sync_agent_reactive(base, agent, "test-token", …)` and the stub directory
  accepts any token, so token age never mattered.
- **The failure is silent by design.** "Couldn't check" must deliver the
  message (spec §1.2), and it did, with no log above debug. The only record
  was the in-memory audit log, which nobody reads by default.
- **Round 1 had a correct but masking explanation.** The 0.57.4 peer explained
  the first failure, so the 06:28 and 07:13 agenta failures on narko weren't
  looked at until round 2.
- **Only a two-machine run over real time shows it.** The check ran within
  minutes of charlie connecting, so one side always had a fresh token.

## Fix

Branch `agent2/wan-verify-fresh-directory-token`:

- `cloud_subscriber::wan_directory_token` loads a fresh shared account token
  (`load_valid_token`, same as the publisher) for each verification and uses
  the connection's token only if none can be loaded. Tests: a fresh token wins
  over the connection's; the connection's is the fallback.
- `wan_verify::get_json` logs a warning with the status (or transport error)
  when the directory refuses or can't be reached, so this can't go silent
  again.

Both machines need the fix: a receiver fails the same way whichever side is
older than 15 minutes into its connection.

## Follow-ups

- **Other users of the connection token** (raised by Opaz, to be checked
  before fixing, in a separate PR): `wan_lease::release` on agent removal
  (a silent 401 would leave the lease held until it expires, fencing a reopened
  agent), and the lease `ensure` / pending fetch fallback when an agent has no
  per-agent credential (the fetch self-heals only by reconnecting). The
  general fix is for `connect_and_run` to hand callees a per-use token loader
  and keep the connect-time token for the WebSocket handshake only.
- Re-run the §5 two-machine check once both machines run a build with this
  fix, with each connection older than 15 minutes, and update the tracking
  doc (C1 is deployed; the check is no longer blocked on it).

## Lessons

- A token captured at connect is only good for the token's lifetime, not the
  connection's. Anything that makes its own HTTP calls should load the token
  per use, as the publisher and relay already did.
- "Couldn't check, deliver anyway" paths need a log line at warn with the
  reason. Here the reason existed but only reached the in-memory audit log.
- When a two-machine check passes one way and fails the other, compare the
  state of the two sides (versions, token and connection ages), not just the
  code.
