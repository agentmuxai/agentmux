# SPEC: a leader spawns workers in another AgentMux: another channel, another host on the LAN, another computer through the cloud

**Status:** proposed, nothing built
**Date:** 2026-10-17
**Author:** Korp@narko, at the operator's request
**Trigger (operator, 2026-10-17):** "by now we should have the worker feature up right? you can spawn an agent and direct it as the leader" … "ah, ok, then record this. The worker spawn needs to happen across channels, hosts, LAN, and cloud" … "lets research it and write a spec to file"
**Builds on:** `SPEC_AGENT_WORKERS_2026_10_15.md` (the local `Agents` `spawn`, Phases A and B shipped, owner Loap-3), `SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md` (`TRUST=channel-verified`), `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` (`TRUST=lan-verified`), `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (`TRUST=wan-verified`, instance keys), `SPEC_CROSS_HOST_AGENT_ACCESS_2026_10_01.md` (proposed: signed, audience-bound requests and approval on the target for acting on another machine), `REPORT_AGENT_OPEN_API_GAP_2026_09_06.md` (cross-channel open left open; "a remote open is closer to execute code on that machine").

---

## 1. Summary

1. **Today a leader can spawn a worker only in its own AgentMux instance** (§2). The worker opens next to the leader, on the leader's machine, with that instance's accounts and code.
2. **That blocks real work.** The motivating case (2026-10-17): a leader in the installed AgentMux had built a change to the browser tools and needed a tester inside its own `task dev` build of that change. A worker spawned in the installed instance would have tested the old code; the operator had to start an agent in the dev window by hand. The same gap blocks a leader on Windows from spawning a builder on a Mac on the LAN, or on a machine away from this network.
3. **One tool, a `target`** (§4.1): `Agents` `spawn` takes where the worker should run: another channel on this machine, a LAN host, or another install of the same account through the cloud. Everything else about a worker stays as it is: a new agent built from the leader's definition, a brief, a leader it reports to.
4. **The target decides, never the request alone** (§4.3). A spawn starts a process with the target owner's accounts, so: on this machine, the leader's own user approves as for a local spawn; on another machine, the user at that machine approves in a window no agent can click, and the machine must have turned remote workers on (off by default).
5. **The request is signed by the leader and bound to the target** (§4.2): the leader's per-agent key for the tier (channel, LAN, WAN), the target instance's id, a nonce and a short expiry. A message, a public LAN key or "same account" is never enough.
6. **The leader link survives the hop** (§4.7): the worker's notes (turn ended, a question, stuck) come back to the leader as signed messages; `Agents` `workers` lists remote workers with where they run; the leader can stop them through the target's own stop path.
7. **Phases** (§5): X1 cross-channel first (it solves the motivating case and reuses what same-machine instances already trust each other with), X2 LAN on the cross-host transport, X3 cloud last.

## 2. What exists (verified on `main`, 2026-10-17)

| Piece | Local (same instance) | Another channel, same machine | LAN host | Cloud (same account) |
|---|---|---|---|---|
| Message | yes | yes, `TRUST=channel-verified` (Ed25519 key in the host-global registry) | yes, `TRUST=lan-verified` (TOFU-pinned per-agent key) | yes, `TRUST=wan-verified` (instance-certified key), queued 30 min |
| Stop | yes | yes: `FleetBulkStop` forwards to the target's own 15 s override banner (`app_api/fleet.rs` `forward_stop_to_shared_channel`) | no | no |
| Swarm broadcast | yes | yes, as the user's own turn (`deliver_forwarded_broadcast`) | yes (#5060), signed by the sending instance, rendered as a verified message, never as the user's turn | no |
| Transcript, conversations | yes | yes, one loopback hop | liveness only | liveness only |
| Open an existing agent | yes (`Agents` `open`) | no | no | no |
| **Spawn a worker** | **yes** (`app_api/agent_spawn.rs`) | **no** | **no** | **no** |

The local spawn's pieces that assume one instance (`agent_spawn.rs`):

- **Who is calling:** `verified_block_id` checks the leader's request against this instance's own per-agent key; another instance holds no key for an agent it didn't start.
- **Admission:** `self_quit::gate` passes only on the user's own turn; otherwise `connections::ask_user` shows a host window anchored to the leader's pane block.
- **Limits:** `MAX_LIVE_WORKERS` (3) counted from local tabs; `leader_lock` is a process mutex.
- **Account:** chosen from this channel's sign-in state.
- **Placement:** to the right of the leader's pane.
- **Leader link:** `agent:worker_of` on the definition, `agent:coached_by` on the pane, worker notes delivered to the leader's local pane (`backend/worker_notes.rs`), `Agents` `workers` reads local tabs.
- **Brief:** sent afterwards by the leader's MCP process with `Message` `send`, by name; across channels this already arrives `channel-verified`.

What doesn't exist at all on the LAN and the cloud is any authenticated command: the LAN surface takes messages with a key every LAN device can read (it is advertised), and the relay only delivers messages to agents already registered. `SPEC_CROSS_HOST_AGENT_ACCESS_2026_10_01.md` proposes the transport this spec needs for the LAN and the cloud (signed, audience-bound requests over an encrypted listener; grants; approval on the target).

## 3. Goals and non-goals

**Goals**
- A leader spawns a worker in another channel on its machine, on another LAN host, or on another install of its account, with the same tool and the same result shape.
- The worker reports to its leader, and the leader can list and stop it, wherever it runs.
- No message, broadcast, LAN key or account membership can make an agent start anywhere without the approval this spec gives.

**Non-goals**
- Moving a running agent between instances (that is a fork or a hand-off, not a spawn).
- Workers on another account's machines (WAN verification is same-account only).
- Workers inside containers or over SSH (`Shell` `connection` already runs commands on SSH hosts, without the agent model).
- Remote control of agents the leader didn't spawn (that is `SPEC_CROSS_HOST_AGENT_ACCESS`'s broader scope).

## 4. Design

### 4.1 The tool

```
Agents { action: "spawn", target?: { channel?: string, host?: string, instance?: string },
         name?, model?, effort?, account?, brief, files?: [path], user_instruction? }
```

- No `target`: today's local spawn, unchanged.
- `channel`: another AgentMux on this machine, by its channel name (as `Agents` `list` shows under `cross_channel`).
- `host`: a LAN peer by host name (as `Agents` `list` shows under `lan`).
- `instance`: another install of this account through the cloud, by the `INSTANCE=<host>~<id8>` label.
- `files`: up to the tier's file limits (`SPEC_JEKT_FILE_TRANSFER_2026_10_05.md`: 100 MB same host, 50 MB LAN, 25 MB cloud per file), sent with the brief. A brief that names an absolute path on the leader's machine gets a warning in the result: the worker can't read it there.
- `Agents` `list` marks each instance or host it shows with `can_spawn` and, when false, why (remote workers turned off there, LAN discovery off, instance not approved, no signed-in account for this provider).
- The result adds `where: { tier, channel | host | instance }`, and for the cloud `status: "pending"` until the target answers (§4.3).

### 4.2 The request

The leader's srv sends the target a **spawn request**, signed with the leader's per-agent key for the tier:

| Tier | Key | Transport |
|---|---|---|
| channel | the leader's Ed25519 channel key, published in the host-global registry | loopback, to the target's local URL with the target's registry auth (as the stop and broadcast forwards do) |
| LAN | the leader's LAN key (`TRUST=lan-verified`'s) | the cross-host transport (`SPEC_CROSS_HOST_AGENT_ACCESS` §6.1): encrypted, never the advertised `lan_key` |
| cloud | the leader's WAN key, certified by its install's instance key | a new relay request type the target's install pulls, alongside its messages |

The signed body: `{ v, leader: {agent, instance_id, host}, target_instance_id, name, model, effort, account_hint, brief_sha256, files: [sha256], nonce, issued_at, expires_at }`. The target refuses a request that is not for its own instance id, is past `expires_at` (2 minutes; 30 minutes for the cloud's queue), reuses a nonce, or whose signature doesn't verify against the key it already knows for that leader. The brief and files follow as the usual signed message, which the target matches to `brief_sha256`.

### 4.3 Who approves

| Tier | Approval |
|---|---|
| channel | **The leader's own user, as for a local spawn**: the leader's turn was started by the user (with `user_instruction` quoted) or the leader's instance asks in its host window. Same machine, same person at the screen, and same-user instances already trust each other with more (the user's own broadcast turn, stop, take-over). The target shows a toast: "Korp (installed AgentMux) started worker Korp-T1 here". |
| LAN | **The target machine's user, always**, in a host window on that machine that no agent can answer, naming the requesting machine, the leader, the worker's name, model and the account it will run on, and the brief's first lines. The target must have **"Accept workers from other machines"** turned on (Settings, default off). The window offers "Allow once" and "Allow this leader for 8 hours" (a grant, `SPEC_CROSS_HOST_AGENT_ACCESS` §6.3); a grant is shown and revocable in Settings. |
| cloud | As for the LAN, plus the requesting install must be **approved** on the target (not merely `new`), and the request waits up to 30 minutes for the target's user; the leader gets `pending`, then a signed answer message (`accepted`, `declined`, `expired`). |

A leader's own approval never stands in for the target's on another machine: the worker would run on the target owner's accounts, files and quota. A turn started by a message, a schedule or a worker's note never spawns without a window on the side that owns the accounts.

### 4.4 Accounts, workspace, placement

- **Account:** chosen on the target from its own sign-in state (`choose_account`'s rule, run there); `account_hint` is a preference (by email), not a credential. The approval window and the result name the account and say it is that machine's.
- **Workspace:** the target's `~/.agentmux/agents/<slug>`, as for any new agent there. Files sent with the brief land in it.
- **Placement:** "right of the leader" doesn't exist on another instance. The worker opens in a tab named `Workers of <leader>@<host>` in the target's focused window (created on first use), so the target's user sees who started what.

### 4.5 The worker's definition

Built on the target from the leader's definition, as locally (`copy_definition(.., as_fork: false)`): the leader's definition travels in the signed request body (its content, skills, provider and flags; never accounts, memory or conversation). Global Memory is the target's own. The worker's brief line names its leader with its host: "You are a worker of Korp@narko (in another AgentMux). Report to it with the agentmux Message tool (action send, to Korp)."

### 4.6 Limits

`MAX_LIVE_WORKERS` counts a leader's workers wherever they run. The leader's instance keeps a small ledger of its remote workers (target, worker id, started, last seen), which counts with its local tabs and is refreshed from the workers' notes and the target's liveness. A target also caps the remote workers it runs at once (default 3, Settings), so one machine's leaders can't fill another's.

### 4.7 The leader link

- `agent:worker_of` on the target records `{agent, instance_id, host, tier}`.
- **Worker notes** (turn ended without a report, a question mid-turn, stuck, exited) are sent by the target's srv as signed messages to the leader, from the worker's own identity, and the leader's AgentMux renders a verified one from a ledger worker as a worker note (as `worker_notes.rs` does locally). An unverified one is an ordinary message.
- **`Agents` `workers`** lists local and remote workers, with `where` and the last time each was heard from.
- **Stop:** the leader stops its remote worker through the target's own stop path: on this machine the existing cross-channel forward with the target's 15 s override banner; on another machine a signed stop request shown in the target's override banner. A worker's own leader stopping it needs no new approval beyond that banner.

### 4.8 Audit

The target records every spawn and stop request it receives (accepted, declined, refused and why), with the requesting instance and leader, in its connections log, and shows the remote workers it runs in a Settings list where each can be stopped.

## 5. Phases

| Phase | Builds | Done when |
|---|---|---|
| X1 | cross-channel: `target.channel`; the signed request over loopback; the leader-side approval; the target toast; tab placement; the remote-worker ledger, notes back as `channel-verified` messages; `Agents` `workers` and stop across channels | a leader in the installed AgentMux spawns a tester in its own `task dev` build, briefs it, gets its notes, lists it and stops it |
| X2 | LAN: on `SPEC_CROSS_HOST_AGENT_ACCESS`'s transport (encrypted listener, signed audience-bound requests, grants); the target setting (off by default), approval window and audit; files with the brief | a leader on Windows spawns a worker on a Mac on the same network, only after its user allows it there |
| X3 | cloud: the relay request type, `pending` and the signed answer, instance approval required; depends on the instance approval UI and retirement (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.8) | the same as X2 for a machine on another network, with the 30-minute window |

## 6. Security

- **A message can't spawn anything.** Admission is the user's own turn or a host window; on another machine always the target's window.
- **The LAN key proves nothing.** It is advertised; spawn requests on the LAN use the cross-host transport and the leader's signature, and the target's setting is off by default.
- **Replay and redirection:** a nonce, a 2-minute expiry (30 for the cloud) and the target's instance id in every signed request.
- **Confused deputy:** a worker can't spawn workers (as locally), and a request can't be relayed onward (`via` hops denied, `SPEC_CROSS_HOST_AGENT_ACCESS` §6.11).
- **Quota and accounts:** the target's window names the account and machine; the target's own cap limits how many remote workers run there.
- **The brief is untrusted text to the worker**, as it is today; the worker's brief line names the remote leader, so the worker knows where its instructions come from.
- **Revocation:** a revoked or unapproved install can't spawn through the cloud; a LAN grant can be withdrawn in Settings at any time and stops the leader's later requests (not its running workers, which the target's user stops in the same list).

## 7. Tests

- **Unit:** request signing and verification (wrong instance, expired, replayed nonce, wrong key, brief hash mismatch); the ledger's counting with local tabs; worker-note rendering only for a verified ledger worker.
- **X1, two channels on one machine:** spawn from the installed build into a `task dev` build; the brief arrives `channel-verified`; a question mid-turn reaches the leader as a worker note; `workers` lists it; stop shows the target's banner; a turn started by a message asks the user instead of spawning.
- **X2, two machines on a LAN:** with the setting off the request is refused; with it on, the target's window appears and "Allow once" starts the worker; a request with only the `lan_key` (no signature) is refused; a replayed request is refused.
- **X3:** a request to an install that is only `new` is refused; an approved install's request waits for the target's user and expires after 30 minutes unanswered.

## 8. Open questions

1. **Cross-channel approval:** is the leader's own user turn enough to start a worker in another channel on the same machine (proposed, matching how same-user instances already trust each other), or should the target channel also ask?
2. **Different builds:** a worker started in another build (a `task dev`, an older release) may have different tools. Should `can_spawn` compare versions and warn?
3. **Ownership:** the worker feature is Loap-3's (`SPEC_AGENT_WORKERS_2026_10_15.md`); X1 touches `agent_spawn.rs`, `worker_notes.rs` and the `Agents` tool. Who builds X1, and in what order with Phase C (the coached-by chip)?
4. **X2 and X3 depend on the cross-host transport,** which is proposed and not started. Build the narrow spawn verb first on it, or the general L1 grant model first?
