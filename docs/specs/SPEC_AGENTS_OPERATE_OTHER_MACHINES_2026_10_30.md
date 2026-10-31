# SPEC: Agents operate other machines: typed remote operations between AgentMux instances, accounts first

**Status:** active — approved by the operator, 2026-10-30 ("yes, spec looks good"), with every recommendation in section 9 (D1-D8). The local half of P1 accounts shipped in PR #5915 (the stable account id, `identity.accounts`, `identity.switch` by stable id, email or `any with quota`, `all limited`). The LAN transport, `app.version` and `app.health` shipped in #5896. Automatic failover shipped after it: an agent that hits a usage limit moves to a signed-in account with quota after its turn, `agent:accountfailover` turns it off, and an account marked "never fail over to this" (right-click it in Connectors) is never chosen. The remote `identity.accounts` and `identity.switch` followed on the LAN transport, with the `host` argument on `Identity`; both need a certificate from an install the receiving machine has approved (a peer known only by its first-seen pin is refused, with a message that says to approve the instance), because the listing carries account emails. `identity.signin.start` and `identity.signin.finish` followed, behind the same approved-install gate: the target runs the provider's login under an account directory it mints and returns the link, and `Identity` `signin` with a `host` shows it to the person where the requesting agent is (for Claude only; other providers follow). The "notification to the machine the operator is using" is not built: the link goes back in the requester's answer, which the person is reading anyway. Not done: the relay message kind, that push notification, and P2-P5. The account id is also in the LAN agent list (`agent_accounts`), which `Agents list` and the Swarm read.
**Date:** 2026-10-30
**Author:** AgentY, at the operator's request: "lets also write a spec that allows agents to set the accounts on other machines, remember agents need superpowers, so fill in other operational gaps you find."
**Tasks:** spec #12438, build #12439 (split into the phase tasks in section 8 once approved).
**Read on:** `origin/main` @ `18e6b20eb`. File:line citations were read on that commit.
**Builds on:** `SPEC_SUPER_AGENT_2026_10_25.md` (the super agent, the countdown and the veto), `SPEC_LAN_JEKT_SIGNED_BY_SRV_2026_10_25.md` and `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (how a sender on another machine is proven), `SPEC_AGENT_DRIVEN_UPGRADES_AND_FLEET_ROLLOUT_2026_10_30.md` (its signed upgrade request becomes one operation of this spec).
**Replaces in part:** `SPEC_CROSS_HOST_AGENT_ACCESS_2026_10_01.md` (proposed, never built). That spec covers running commands and editing files on another machine with a person approving the risky steps; it predates the operator's 2026-10-25 rule that agents can do anything except speak in the operator's name. This spec does not cover shell or files (section 7, item 9).

## 0. Summary

Today an agent can do almost anything to the AgentMux it runs in, and almost nothing to the AgentMux on another machine. The only thing that crosses machines is a text message that some agent over there has to read and act on.

This spec adds **remote operations**: a typed request one AgentMux sends another, on behalf of one of its agents, which the receiving AgentMux carries out itself. The rule is simple:

- **An agent can do on another machine what it can do on its own**, no more and no less, when its machine is proven (`lan-verified`, or `wan-verified` from an approved install of the same account).
- **What is sensitive locally is sensitive remotely**, and goes through the *receiving* machine's super agent and its countdown with a veto, as it does today.
- **Everything is recorded on both machines, as the agent that did it.** Nothing is ever presented as the operator's words.

The first operations are the account ones: list another machine's accounts with their quota, and move its agents to an account that has quota. The rest of the list (section 4.3) closes the other gaps the research found: start and stop agents, seats, Global Memory, the task board, upgrade, reclaim, disk, logs.

It also fixes four things that hurt on their own (section 5): a message to an out-of-quota agent on another machine is dropped, not held; agents can't see which accounts have quota; the same account has a different label on every machine; and nothing moves an agent off an exhausted account by itself.

## 1. Why

When a plan's usage limit resets on one account and not on another, a person with agents on several machines has to move each machine's agents to the account that has quota. Today that cannot be done from one place:

- `Identity switch` is a route on the agent's own AgentMux only (`crates/srv/src/server/http_self.rs`). There is no way to ask another AgentMux to do it, and if every agent on a machine is out of quota, nobody there can do it either.
- The same provider account is labelled differently on each machine (`email ·1` to `·4`, by sign-in count), so an account named by its label on one machine is not found on another.
- A message to an out-of-quota agent on another machine is refused and not kept, where on the sender's own machine it would be held and delivered after the move.
- Another machine keeps showing an agent as limited after its account was changed, until the agent itself is moved.

Each of these is a few calls for an agent that has the reach, and a long manual job for a person without it.

## 2. What exists today (verified on `18e6b20eb`)

| Area | Fact | Where |
|---|---|---|
| Account switch | Any agent, or the operator's window, moves an agent on **this** AgentMux to another signed-in account: `after_turn` queues it in `agent:pending_account`, `now` restarts the pane; old limits cleared; audit row names the actor | `server/account_episodes/switch.rs:168-228`; route `http_self.rs:97` |
| Account labels | `email ·N` is numbered per machine when several sign-ins share an email | `account_numbering.rs` |
| Quota state | An agent is `Limited` while its `agent:last_failure` is a usage limit whose reset is in the future. No automatic move to another account | `backend/agent_state.rs:138-144` |
| Quota view | `identity.account.availability` lists open limit episodes, **operator's window only**; moving agents through that RPC is operator-only too | `account_episodes/mod.rs:621`, `:629` |
| Message hold | Held 24 h only when the request came with the full local key, tier `host`, not from cron. A LAN, cross-channel or cron delivery is refused with `refused_reason` and `retry_when`, and not kept | `server/jekt_hold.rs:318-327` |
| LAN routes | `inject`, `inject-files`, `agent`, `agent-names`, `agent/holding`, `fleet` (+ events). All are about messages and directory, none performs an action | `server/routes.rs:108-138` |
| LAN proof | Per-agent Ed25519 keys, pinned on first use; srv signs for an agent that holds no key. `lan_key` is broadcast and proves nothing | `jekt_sign.rs`, `agent_lan_keys.rs`, `lan_peer_pubkey_pins.rs`; `SPEC_LAN_JEKT_SIGNED_BY_SRV_2026_10_25.md` |
| WAN proof | Instance key, per-agent certificates, account directory, instance status `approved` / `new` / `revoked`. Only messages travel | `muxbus/wan_verify.rs` |
| Sensitive ops | One table (`quit_agentmux`, `upgrade`, `widget_install`, `desktop_control_grant`, `vault_use`, …); the super agent approves; a countdown with the person's veto | `backend/sensitive_ops.rs`, `sagas/sensitive_window.rs` |
| Super agent | Must run in the AgentMux that received the request. A request can't come from another machine | `SPEC_SUPER_AGENT_2026_10_25.md` §3-4 |
| Fleet status | Each agent's state (working, idle, limited, signed out, …) is published to LAN peers on change. No quota or account detail | `backend/fleet_feed.rs:55-72` |
| Global Memory, task board, schedules, skills | Per machine. A Global Memory change made on one machine is copied to the others by hand, by each machine's steward | MuxAPI `Memory`, `Tasks`, `Schedule`, `Skills` |

The one structural fact: **every interaction between machines is a text message injected into an agent.** There is no typed request that a receiving AgentMux acts on itself.

## 3. Principles

1. **Same power, any machine.** An agent's reach on another proven machine equals its reach on its own. No operation is added that an agent can't already do locally; none is removed.
2. **Proof, not location.** A remote operation needs a proven requesting machine: `lan-verified`, or `wan-verified` with instance status `approved`. `network-claimed`, `self-declared`, `unverified`, `SIG=invalid` and `revoked` are refused, never downgraded to a prompt. The broadcast `lan_key` never authorizes anything.
3. **The receiving machine decides, with its own rules.** It runs the operation through the same code path as a local call, with the same sensitive-op table, the same super agent and the same countdown, shown on its own screen.
4. **Typed, not text.** The receiving AgentMux acts on a structured request. Text from the request is never injected into one of its agents as an instruction, and banners are built from the structured fields, never from the requester's words (as `SPEC_SUPER_AGENT` §2 already requires).
5. **Attribution is the boundary.** The actor is always `agent@machine` on both machines' audit trails. A remote operation is never recorded or shown as the operator's.
6. **Any agent, not only a leader.** Consistent with the upgrade spec's D7 and the 2026-10-25 rule. Seats route work; they don't gate capability.

## 4. Design

### 4.1 The request

One envelope for every operation:

```
RemoteOp {
  v: 1,
  op: "identity.switch",                 // from the table in 4.3
  params: { ... },                       // typed per op
  requester: { agent_uid, slug, instance_id },
  audience: "<target instance id>",      // the one machine it is valid on
  ts, nonce,                             // replay protection, a few seconds of clock window
  reason: "<short, the requester's words, shown as theirs>"
}
```

- **Signed at the origin by srv**, the way LAN and WAN messages already are: over LAN with the requesting agent's LAN key (srv signs for it, `SPEC_LAN_JEKT_SIGNED_BY_SRV`); over the relay with the instance key that certifies the agent. The receiver verifies the signature, the audience, the timestamp and the nonce before anything else.
- **Transport.** LAN: a new route `POST /agentmux/remote/op` on the existing peer listener, served only for a verified signature (never for `lan_key`), answering synchronously with the result. WAN: a new structured relay message kind that the receiving srv consumes itself (never injected into an agent), answering with a signed result message.
- **Result**: `{status: done | queued | pending_veto | refused | failed, result, audit_id, refused_reason?}`. `pending_veto` means a countdown is running on the target; the final outcome follows as a second result.
- **Idempotent by `nonce`**: a retried request returns the first result.

### 4.2 Authorization

1. Verify the requester's machine (principle 2). Refuse otherwise, with the reason.
2. Resolve the requester as a **known remote agent** (`agent@machine`). It holds no local token; the receiving srv runs the operation with a caller context whose actor is that remote agent, and whose rights are an ordinary agent's (never `operator_only`, never the instance key).
3. Look the op up in the receiving machine's sensitive-op table. If it is sensitive there, file it as a `Sensitive` request on that machine: its super agent reviews it, then the countdown runs with the veto (section 9, D3, on where the banner shows). If no super agent answers, the existing fallback applies (`SPEC_SUPER_AGENT` §6).
4. Run the op's own code path, unchanged.
5. Write one audit row on each machine (4.7).

A machine can turn remote operations off (Settings, and the steward's chores): one switch, which refuses every remote op with `refused_reason: remote_ops_off` (D1).

### 4.3 The operations

Version 1. "Local rule" means: whatever the same call does on the agent's own machine today.

| Op | What it does on the target | Sensitive there? | Phase |
|---|---|---|---|
| `identity.accounts` | Signed-in accounts with stable id, provider, email, label, sign-in state, **quota state and reset time**, and which agents use each | no (read), but only for a caller whose install this machine has approved: it returns account emails | P1 |
| `identity.switch` | Move an agent (or `"all limited"`) to an account named by stable id, or to `"any with quota"` of the same provider; `after_turn` or `now` | no (local rule) | P1 |
| `agents.list`, `agents.conversations` | Already cross-machine for names and status; adds the full host entry (runtime, seat, availability) | no | P1 |
| `agents.start`, `agents.open` | Start an agent there (into a tab the target picks, never the operator's active one) | no | P3 |
| `agents.stop` | Stop an agent there | the 15 s keep-running window, as locally | P3 |
| `agents.set_seat`, `agents.env_get`, `agents.env_set` | As locally | no | P3 |
| `tasks.enqueue`, `tasks.list` | File a task on that machine's board for one of its agents; read its board | no | P3 |
| `memory.global.put`, `memory.global.remove`, `memory.global.list` | Global Memory on that machine (and the fleet sync of 4.6 uses these) | no | P4 |
| `skills.import`, `skills.bind` | As locally | no | P4 |
| `app.version`, `app.health` | Version, build, channel, uptime, MuxBus per channel, LAN peers, disk free, last errors (the upgrade spec's health report, on demand) | no | P1 |
| `app.logs.tail` | The last N lines of AgentMux's own srv and host logs, secrets masked | no | P5 |
| `app.history_reclaim` | Request Reclaim at the next start | no (local rule) | P5 |
| `app.disk.report`, `app.disk.cleanup` | AgentMux's own leftovers only (old versions, dev homes, build caches it created), by the disk-cleanup skill's rules | cleanup: 30 s window | P5 |
| `app.upgrade` | The upgrade spec's request, carried by this transport instead of its own message type | 2 min, as locally | P5 (with #11443) |
| `app.restart` | Only if the operator approves the one-restart-path proposal for the upgrade spec | 2 min | P5 |

Not in this spec: running commands, reading or writing arbitrary files, `UI` clicks, `Desktop` and `Browser` on another machine, and anything that reads a Vault value. Section 7, item 9.

### 4.4 Accounts across machines

- **A stable account id.** Each signed-in account carries the provider's own identity where it has one (for Claude, the OAuth account and organization ids the CLI reports), plus the email. Two machines' sign-ins of the same provider account share the stable id; labels (`·2`, `·4`) stay a local display only. `account` in `identity.switch` (local and remote) accepts the stable id, the email when it names exactly one account with quota, or `"any with quota"`.
- **Quota visible to agents.** `identity.accounts` (local and remote) shows each account's quota state and reset time. The underlying `identity.account.availability` becomes readable by agents; only labels and states, never a credential. The fleet feed adds each agent's account stable id, so the Swarm and `Agents list` can show "out until …, on account X" for every machine.
- **Automatic failover** (D4). When an agent hits a usage limit and another signed-in account of the same provider on the same machine has quota, AgentMux moves it after the turn and tells the agent and its lead in one note. The operator can mark an account "never fail over to this", and the whole behaviour is one setting.
- **Signing in another machine** (D6). OAuth needs a person in a browser, and credentials never travel between machines. What an agent *can* do: ask the target to start a sign-in (`identity.signin.start`, P6). The target opens the provider's login and returns the link (and `identity.signin.finish` waits for the sign-in to end, taking the code the person was shown if the browser can't finish it); the agent shows the link to the person where they are, and later a notification can send it to the machine the operator is using now ("Sign machine X in to Claude"). The person completes it there; the token is written only on the target, and a sign-in belongs to the caller that started it. Same for a MuxBus sign-in lost on upgrade.

### 4.5 Messages to an agent that can't work, from another machine

`hold_applies` (`jekt_hold.rs:318`) extends to a verified LAN or WAN delivery (`lan-verified`, or `wan-verified` from an approved install). The message is held on the receiving machine for 24 h and delivered on the reset or the account move, exactly like a local one, and the sender is told `HELD for X (usage_limit, until …)` instead of a refusal. Unverified deliveries keep today's refusal.

### 4.6 One Global Memory for the fleet

A Global Memory entry gets a scope: **fleet** (the default for new entries, D5) or **this machine**. A fleet entry written on any machine is pushed with `memory.global.put` to every proven same-account machine; each keeps its own history (Memory's history and revert already exist), and the last write wins. A machine that was offline catches up on its next contact, newest first. Seats follow the same path, so "set the seat on the agent's own machine; the steward does it" becomes one call.

### 4.7 Audit

Each remote operation writes, through `log_fleet_action_audit`, `remote.request` on the requester's machine and `remote.<op>` on the target, with requester `agent@machine`, the op, a parameter summary, the verification (`lan-verified` / `wan-verified`), the outcome and, for a sensitive op, the super agent and the veto or run. Both show in Warden's Audit pane.

### 4.8 MCP surface

`Identity`, `Agents`, `Tasks`, `Memory` (global), `Skills` and `App` gain an optional `host` argument: a machine's name as `Agents list` shows it, or its instance id. Absent means this machine, as today. It must be named on each call, never inherited from an earlier call, so a local action can't silently become remote. Results say which machine answered. A name that matches more than one machine is an error, never a guess.

## 5. The four problems that hurt on their own

Each is fixed by a part of section 4 and can ship early:

1. **Messages dropped across machines**: 4.5. Small, and it stops losing work today.
2. **Agents can't see quota**: 4.4, the read side. Lets a leader see the fleet's quota in one call.
3. **One account, a different label per machine**: 4.4, the stable id.
4. **Nothing moves an agent off an exhausted account**: 4.4, failover.

## 6. Security

The operator's model (2026-10-25): agents may do anything except put words in the operator's mouth; sensitive operations go to one super agent with a human override window. Within that:

| Threat | Control |
|---|---|
| A forged or unproven machine sends a request | Signature over audience, op, params hash, ts, nonce; `lan-verified` or approved `wan-verified` only; `lan_key` never accepted |
| Replay, or a request reused on another machine | Nonce store and short clock window; `audience` binds it to one machine |
| A hostile message talks an agent into a remote op | A remote op is a tool call by an agent, as a local op is; the target never injects request text into its agents; sensitive ops still face the super agent and the person's veto |
| The operator's voice is forged | No op writes anything as the operator; banners name the requesting agent and machine; `operator_only` stays only on the person's veto and approve-now |
| A credential leaks between machines | No op returns a token or a Vault value; sign-in happens on the target, the person completes it |
| A compromised machine abuses its reach | Instance `revoked` refuses everything from it; each machine's off switch; both sides keep the audit row |

Nothing here weakens a jekt's `TRUST`, `TIER` or `ESCALATE` rules: remote operations are a separate, typed channel.

## 7. Other operational gaps found, and where each goes

1. **The launcher's own upgrade dialog** that no agent can click: the upgrade spec (#11443), and `app.upgrade` here.
2. **A message dropped across machines**: 4.5 (P2).
3. **Quota invisible to agents; labels differ per machine; no failover**: 4.4 (P1, P6).
4. **Global Memory and seats copied by hand to every machine**: 4.6 (P4).
5. **The task board is per machine**: `tasks.enqueue` and `tasks.list` remotely (P3). A single fleet board stays a later spec.
6. **Starting a worker on another machine** (#200): `agents.start` covers existing agents (P3); spawning a new worker remotely follows once P3 is in.
7. **Signing a machine in without walking to it**: 4.4 (P6). The person still completes the login.
8. **The steward's routine checks** (agents back after an upgrade, MuxBus per channel, LAN peers, disk): `app.health` and `app.disk.*`, callable by any agent, so a leader can check all four machines without waking four stewards.
9. **Commands, files, UI and desktop on another machine**: not here. The 2026-10-01 access spec should be rewritten under the 2026-10-25 rule (same power as locally, super agent and veto for the sensitive part) as its own spec, after P1 proves the transport (D8).
10. **Browser handoffs** (sign-in, 2FA, CAPTCHA, payment) stay the person's. What changes: the handoff notice reaches the machine the person is on (the same notification path as 4.4's sign-in link).

## 8. Phases

One or two PRs per phase, each with a changeset; owners are the seats that own the area.

| Phase | Scope | Owner |
|---|---|---|
| **P1 transport and accounts** | The envelope, signing and verification, the LAN route and the relay message kind, audit rows, the off switch; `identity.accounts`, `identity.switch` (local and remote), the stable account id, agents can read availability, `agents.list` full entry, `app.version` and `app.health`; the `host` argument on `Identity` and `App` | transport: Lazo; accounts: Agent5; security review: AgentY |
| **P2 hold across machines** | 4.5 | Manoz (agent availability) |
| **P3 agents and tasks** | `agents.start`, `open`, `stop`, `set_seat`, `env_*`; `tasks.enqueue`, `tasks.list`; `host` on `Agents` and `Tasks` | Manoz, Naki-2 (Kanban) |
| **P4 fleet memory** | Fleet scope for Global Memory and seats, catch-up, skills | Clamk |
| **P5 app operations** | `app.logs.tail`, `app.history_reclaim`, `app.disk.*`, `app.upgrade` with #11443, `app.restart` if approved | QTest (upgrade), Loap-3 (disk, reclaim), Posa (Windows checks) |
| **P6 failover and sign-in** | Automatic failover; `identity.signin.start` and the cross-machine notification | Agent5, QTest |

P1 and P2 can start together. P2 is small and worth shipping first.

## 9. Decisions for the operator

| # | Question | Recommendation |
|---|---|---|
| **D1** | Remote operations from any proven same-account machine: on by default, with a per-machine off switch? | **Yes.** Off by default would put a person back in the loop this spec removes. |
| **D2** | Which remote ops are sensitive? | **Exactly the ones that are sensitive locally** (stop: 15 s; disk cleanup: 30 s; upgrade and restart: 2 min). An account switch is not sensitive locally and isn't remotely. |
| **D3** | Whose super agent reviews, and where does the countdown show? | **The target machine's super agent; the banner shows on the target and on the machine you are using now**, so you can veto from wherever you are. |
| **D4** | Automatic failover to another account with quota on the same machine? | **On**, after the turn, one note to the agent and its lead; per-account "never fail over to this". |
| **D5** | Global Memory entries fleet-wide by default? | **Yes**, with a "this machine only" scope for machine-specific entries. |
| **D6** | Sign-in of another machine: a link sent to the machine you are on? | **Yes.** The token stays on the target. |
| **D7** | Hold a verified message from another machine for 24 h, like a local one? | **Yes.** |
| **D8** | Rewrite the 2026-10-01 command-and-file access spec under the 2026-10-25 rule as a follow-up, after P1? | **Yes, after P1.** |

## 10. Tests (written to fail without the change)

- A remote `identity.switch` from a `lan-verified` machine moves the agent on the target and writes both audit rows; the same request signed with only the `lan_key`, or replayed, or sent to another machine (`audience`), is refused.
- A request from a `wan-verified` instance whose status is `new` or `revoked` is refused, never prompted.
- `identity.switch` with `"any with quota"` picks an account of the same provider whose quota state is ok, and refuses with the list when none has quota.
- The same provider account signed in on two machines shows the same stable id on both, and `identity.switch` by that id resolves on each machine to its own sign-in.
- A remote `agents.stop` opens the target's 15 s window; the person's Keep running cancels it; a remote `app.upgrade` faces the target's super agent and the 2 min veto.
- A verified LAN message to a `limited` agent is held and delivered after an account move; an unverified one is refused as today.
- No remote op writes a row, banner or message that names the operator as its author.
- The target never injects a request's `reason` into one of its agents.
- A fleet Global Memory entry written on one machine appears on the others within a minute, and on a machine that was offline at its next contact.

## 11. Not verified

- Which stable account fields each provider exposes (the Claude CLI reports account and organization ids; Codex, Kimi, Qwen, Gemini, Antigravity need checking in P1).
- Whether the relay's existing message kinds can carry a typed payload the receiving srv consumes without reaching an agent, or need a new kind (P1).
- How long the fleet feed takes to show a changed account (event-driven per `fleet_feed.rs`; not measured).

Agent: AgentY@narko
