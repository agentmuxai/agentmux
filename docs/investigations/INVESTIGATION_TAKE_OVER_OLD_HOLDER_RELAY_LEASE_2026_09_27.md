# Investigation — Take over fails against an older instance that still holds the relay lease

**Date:** 2026-09-27
**Author:** AgentY (agent, narko), at operator request
**Status:** active — root cause found; fix in #3921 (desktop Take over) and a cloud-side change (`POST /agents/lease/take`)
**Related:** `docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` (§4.5 relay lease, §4.6 Take over),
#3742 (Take over), #3897 (relay subscription leak), #3899 (Take over reaches a relay-only holder),
the cloud-side agent leases.

## 1. What the operator saw

On narko, Agent2 ran in the 0.57.6 portable (channel `local-main-b28b7a-4a884b76`). The operator
closed Agent2's pane there and opened Agent2 in a 0.58.0 portable (`8a973e456`, channel
`local-main-b28b7a-2017284e`). It started, then showed "running in another AgentMux instance".
Take over → confirm errored again, every time. The same happened to AgentA on Area54 when moving
to 0.57.8/0.58.0. The new tray icon makes it common: closing a window no longer quits the old
instance, so it keeps running.

## 2. Timeline (UTC, from both srv logs)

| Time | Instance | Event |
|---|---|---|
| 03:48:12.0 | 0.57.6 | Agent2's pane closes (`registry: entry changed hands … skipping remove`, `subagent watcher channel closed`). |
| 03:48:44.9 | 0.58.0 | `agent_admission.denied: the muxbus relay says another instance holds this agent` — holder `this computer, channel local-main-b28b7a-4a884b76, v0.57.6`. |
| 03:48:50.9 | 0.58.0 | Classified `LiveElsewhere` → the pane offers Take over. |
| 03:48:58.121 | 0.58.0 | `agent_admission.takeover: requesting release from the holder` (`holder_channel` = 0.57.6's). |
| 03:48:58.122 | 0.57.6 | `agent_admission.takeover: release handled`, **`released: 0`**. |
| 03:48:58.580 | 0.58.0 | `agent_admission.granted` (local lease epoch 5); Agent2 spawns. |
| 03:49:12.441 | 0.58.0 | `agent_admission.fenced: the muxbus relay says another instance holds this agent — stopping this one`, holder still 0.57.6. |
| 03:49:18.6 | 0.58.0 | Every retry: `denied`, same holder. |

## 3. Root cause

- **The old holder never lets go of the relay lease.** On builds before #3897/#3899, an agent's
  cloud subscription outlives its panes. The subscription's lease tick keeps renewing the agent's
  relay lease (`/agents/lease/renew` every ~20 s, TTL 60 s) for as long as that instance runs.
- **Its release handler only stops panes.** 0.57.6's `POST /agentmux/agent/release` found no pane
  (`released: 0`) and answered success. #3899 added the holder-side step (drop the subscription
  and await the relay release, `release_agent_now`), but 0.57.6 doesn't have it. **No released
  build up to 0.57.8 does.**
- **The requester can't free it.** 0.58.0 did everything #3899 asks of the requester. But
  `wait_until_free` checks only the local lease file and the process probe, not the relay. The
  relay's `renew` and `release` act only for the holder instance, and `claim` refuses while a live
  lease is held by someone else. So admission was granted, the lease tick found the relay still
  naming 0.57.6, and the new pane was fenced 14 s later.

In short, #3899 is a holder-side fix. It can't help while the holder is an older build, and every
upgrade from a build older than #3899 hits this as long as the old instance stays running (e.g. in
the tray).

## 4. Workaround

Quit the old instance completely (tray → Quit, not closing the window). Its relay lease lapses
within 60 s, then Retry in the new instance works.

## 5. Fix

Take over must be able to move the relay lease itself, not only ask the holder to.

- **agentmux-cloud:** `POST /agents/lease/take`. Same body as a claim. It grants the lease to the
  caller even while another instance holds it live, **only when the live lease's recorded
  `account_user_id` equals the caller's account** (non-empty). The write is conditional on the
  row as read (a lost race is re-read once, as `claim` does) and bumps the epoch. Otherwise it's
  refused like a claim (409 `held_by_other`).
  - The old holder's next `renew` fails (`holder_instance` no longer matches). Its `claim` is
    refused while the new holder renews. So it can't win the agent back: its pulls are fenced
    and its CLI (if any) is stopped by its own lease tick.
- **Desktop Take over (`server/agent_takeover.rs`):** after the holder answers and the local
  lease is free, claim the relay lease now. If the relay still names an instance **on this
  computer**, call `take` once. Another computer keeps the existing refusal. A relay without the
  route, or an unreachable one, fails open as before.

Why same-account and explicit only: the lease exists so two installs don't both run one agent. An
install on the same account can already hold any of that account's agents; `take` only lets the
user's explicit Take over resolve which one, instead of waiting for an old install to exit.

## 6. Verification plan

- Unit: the lease store's `take` (free, held by self, held by same account → moved with epoch+1,
  other account → refused, empty account → refused, lost race), route tests, desktop `take`
  outcome mapping.
- Live on narko: 0.57.6 (this instance, old holder) keeps a relay lease on an agent whose pane is
  closed. A build with the desktop fix opens that agent and uses Take over. Expected: the pane
  runs and isn't fenced; the 0.57.6 log shows its renewals refused.
