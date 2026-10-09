# SPEC: Cross-host agent access — any agent can work on any AgentMux host, with gated privilege

**Date:** 2026-10-01
**Status:** proposed — research and design; nothing implemented. Asks for decisions in section 11 before any code.
**Author:** AgentX (narko), at the owner's request
**Affects:** a new remote-access plane in `crates/srv/` (new routes and a transport, separate from the jekt routes in `server/routes.rs`), `crates/mcp/src/tool_schemas.rs` (an optional `host` target), a per-OS elevation helper, an approval window in `crates/cef/`, an audit store, the status bar.
**Builds on:** `docs/specs/SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md` (this spec is the concrete design for its "remote invocation" part), `docs/specs/SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md`, `docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`, `docs/specs/SPEC_LAN_FIREWALL_SETUP_2026_10_01.md`.
**Related:** `docs/specs/SPEC_AGENT_HOST_CONTEXT_2026_04_14.md` (defers remote tool execution as "a separate, larger feature"; this is that feature), `docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md`, `docs/specs/SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md`, `docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`.

## 1. Goal

AgentMux already connects agents to each other (jekts, LAN, cloud). This spec lets an agent on one host **do work on another host that runs AgentMux** — run commands, hold an interactive shell, read and write files, and, when truly needed, act as root or administrator — through one clean API, with the security properties a privileged-access system is expected to have.

Success looks like this: when an agent needs something done on Area54, on a Linux VM, or on a Mac, it calls one tool with a target host; the owner of that host has already decided, once, how much that agent may do there; anything beyond that stops for a human; and everything that happened is on record. The hand-built SSH network and its stored logins are no longer needed.

Non-goals: a general VPN, a replacement for configuration management, or access to hosts that do not run AgentMux. Out of scope for the first phases: cross-account access.

## 2. Why now

- Work this session needed it. Diagnosing one remote machine took roughly a dozen messages through that machine's own agent, some of them queued for minutes on the cloud relay, plus a human relaying one step. A direct, audited way to inspect that host would have taken a few minutes.
- The next test (a clean Windows VM for the firewall work) needs a pristine machine that can be reverted and driven, which is the same capability.
- The ad hoc alternative, a separate SSH network with logins in a secrets store, is a second identity system with its own keys, no per-action approval, no shared audit, and nothing that ties an action to the agent that asked for it.

## 3. What exists today (verified in the code, 2026-10-01)

| Area | Fact | Consequence for this design |
|---|---|---|
| Remote execution | **None, on any tier.** The multi-tier spec says so explicitly: "Muxbus is a message bus, not an RPC bus". The LAN router serves only `/health`, a webhook, `/agentmux/reactive/inject`, `/agentmux/reactive/agent`, `/agentmux/reactive/agent-names`, `/agentmux/agent/holding` (`server/routes.rs`) | Everything here is new; there is no existing exec surface to harden, which is an advantage |
| LAN credential | The `lan_key` is **broadcast in the mDNS TXT record** and in the UDP probe answer (`lan_discovery.rs`). Peer URLs are plain `http://`; discovery is unauthenticated | **The `lan_key` proves nothing and must never gate anything privileged.** Remote access needs real per-request identity and an encrypted channel |
| LAN identity | Per-agent Ed25519 keys sign LAN jekts; receivers pin public keys trust-on-first-use (`jekt_sign.rs`, `agent_lan_keys.rs`, `lan_peer_pubkey_pins.rs`) | The identity primitive exists and can be reused |
| WAN identity | Per-install Ed25519 instance key, per-agent certs, an account key directory, `approved` / `new` / `revoked` instance status (`SPEC_WAN_JEKT_VERIFICATION`, `muxbus/wan_verify.rs`). Verification covers jekt text only today | The same-account chain "instance id ↔ instance key ↔ agent cert ↔ agent signature" is the trust root to extend |
| Local privilege | Every local agent holds the instance `auth_key`, which authorizes every loopback route (host shell, any-file read, writes under `$HOME`); permission mode defaults to `bypass`. `X-Agent-Token` is attribution only | An agent is already fully trusted **on its own host**. The remote plane must not be weaker than that, and must not hand a remote caller the instance `auth_key` |
| Approval precedent | The CEF `credential_broker` and memory-adoption windows open a host-process window whose result srv accepts only on a host channel secured by `AGENTMUX_HOST_REG_SECRET`. They are deliberately not an in-page modal, because agents can drive modals through `UIClick` | This is the pattern for human approval here |
| Elevation precedent | `pkexec` with a fixed command catalog in `server/system_install_handlers.rs`; a Windows UAC helper is designed but not built (`SPEC_LAN_FIREWALL_SETUP`) | The elevation helper is a new, small component per OS |
| Audit | A jekt audit log and an event log exist; **nothing records command execution** | An audit store is a prerequisite, not an add-on |
| Server-side authorization | Cloud-side per-agent authorization is tracked in the private cloud repo; `conversation_trust_grant_check` is always false for tier `wan` | The multi-tier spec sequences remote invocation last, gated on enforced per-agent authorization. Section 9 keeps that gate |
| MCP tools | `Shell`, `PtyShell*` take `cmd`, `cwd`, `env`, `rows`, `cols` and **no host parameter**; `FleetBroadcast` is a loop of `SendMessage` | An optional target is a compatible addition |

## 4. What the research says

Sources are listed in section 12. Vendor blogs are marked as such; the primary documents are named where I could reach them.

1. **No standing privilege; elevate per task, for a bounded time.** Just-in-time access grants rights for one task and window and revokes them automatically (NIST SP 800-53 AC-6 least privilege, AC-6(9) log privileged function use; SP 800-207 zero trust: decide per request from context). The weak points named repeatedly are a weak approval path, an over-long window, and failed revocation, which turn temporary access into standing access.
2. **The credential should carry the grant and expire.** Teleport issues short-lived certificates that carry roles; an approved request updates them for the approved window and they revert at expiry. Users cannot approve their own request; dual approval is possible. SSH certificate authorities (Smallstep, OpenSSH) point the same way: hours-to-a-day lifetimes, narrow principals, and short life used in place of revocation lists, with the CA key as the crown jewel.
3. **Re-authenticate for the risky case.** Tailscale SSH has an `accept` action and a `check` action; `check` requires a recent fresh login and is recommended for root. Recording is configured separately and can be enforced. Note: `check` does not suit unattended tools, which is the same tension agents have.
4. **The target connects out; the broker sees nothing it should not.** AWS Session Manager opens no inbound ports: an agent on the target dials the service, and access is decided by IAM. Logging is **not on by default**; sessions tunneled through port forwarding are not logged. Lesson: turn the log on by default, and do not offer a side path that bypasses it.
5. **Privileged-action guidance for AI agents.** OWASP's agentic risks name tool misuse and identity and privilege abuse; the cheat sheet's mitigations are least agency, per-tool profiles, **human confirmation before destructive or high-impact actions, approvals that are parameter-bound, unexpired and single-use**, sandboxes and egress limits. It also names the counter-risk (ASI09): approvals get rubber-stamped, so prompts must be clear and specific.
6. **Never pass a credential through.** The MCP authorization specification forbids token passthrough: a server must not accept or forward a token not issued for it, must validate the audience, and must get its own token for each upstream. This is the confused-deputy rule. It maps directly to: a grant is bound to one target host and is never reusable on another.
7. **Delegation should only narrow.** Capability tokens (macaroons, Biscuit, and an IETF draft for agent delegation chains) can be restricted by any holder and never widened, with a mandatory time bound at each hop. The gap analysis notes that time limits alone suit fast agents poorly, so a **use count** is a useful second bound.
8. **Allowlists beat blacklists, and structured beats textual.** Windows JEA exposes only listed commands and parameters in a constrained runspace under a temporary virtual account, with transcripts. sudoers command lines are the fragile counterpart: a permitted editor, pager or interpreter is a shell. Both fail when the allowlist contains a command that can run arbitrary code.
9. **Privileged helpers: narrow API, authenticate the caller every time.** macOS: an SMAppService helper that verifies the peer's code signature from the XPC audit token. Windows: a service or elevated helper with a locked-down pipe ACL, where impersonation misuse and pipe squatting are the recurring bugs. Linux: polkit, which authorizes per action, not per session, and where the classic bug is a missed check on one method. Common rules: treat the client as untrusted, authenticate on every request, keep the privileged API narrow, validate every path and argument.

## 5. Principles

Each principle is a rule the design below must satisfy.

- **P1 Deny by default.** A host with no policy for a caller allows nothing. Remote access is off until the host's owner turns it on.
- **P2 Identity is cryptographic, per request, and bound to the target.** No shared secret, no network location, no "same account" label grants anything by itself.
- **P3 Least agency.** Three levels (section 6.2), not one switch. Most work needs the lowest.
- **P4 No standing privilege.** Grants expire in minutes to hours and may carry a use count. Elevation is per action.
- **P5 A human decides the dangerous things, on the host being changed, and an agent cannot reach that decision.**
- **P6 Approvals describe exactly what will run.** What was approved is what executes, byte for byte.
- **P7 Everything is recorded, by default, in a place the actor cannot edit.**
- **P8 No weaker than local, no stronger by default.** Locally an agent already has the instance's full power. Remote access must not exceed that, and a remote caller never receives the instance `auth_key`.
- **P9 One kill switch.** The owner can end all remote access to a host instantly.

## 6. Design

### 6.1 Identity and channel

- **Caller identity** is the pair (instance, agent): the instance Ed25519 key and the agent key it certifies. LAN uses the existing per-agent keys and pins; same-account WAN uses the existing certificate chain and instance status. A request from a `revoked` instance, or one that does not verify, is refused outright (never downgraded to a prompt).
- **Every request is signed** over: a version, the **audience** (target instance id), the caller's identity, a method name, a hash of the parameters, a timestamp and a single-use nonce. The target rejects wrong audience, stale timestamp (clock window of a few seconds) and repeated nonce. This makes a captured request useless against any other host and for a second time.
- **Confidentiality and integrity.** The LAN transport today is plaintext HTTP, which is unacceptable for command output and file contents. The remote plane gets its **own listener and its own encrypted channel**, separate from the jekt routes: mutual authentication with the pinned instance keys (TLS 1.3 with raw public keys or self-signed certificates pinned by key, the way Syncthing identifies devices; the choice between that and a Noise-based channel is an open question in section 11). The jekt routes and the `lan_key` are untouched and never serve a remote-access verb.
- **WAN.** The target **dials out** to the relay and holds an outbound stream (the Session Manager model), so no inbound port is needed on a laptop behind NAT. The relay forwards ciphertext only; the end-to-end channel is the same as on LAN, so the relay cannot read or alter a session. The relay's account directory is a source of public keys and nothing more.

### 6.2 Access levels

| Level | Meaning | Examples | Default policy for an approved same-account instance |
|---|---|---|---|
| **L0 observe** | Read-only, within named scopes. No side effects | host info, process list, service status, tail a log, read a file inside the workspace or a configured scope, list a directory | `auto` (no prompt), subject to rate limits |
| **L1 act as the host's AgentMux user** | The same power a local agent has: run a command, hold a shell, write files, start and stop that user's processes | `exec`, `pty`, write or patch a file under `$HOME` | `ask` the first time per (caller, host, scope), then a time-bound grant; `auto` only if the owner opts in |
| **L2 elevated** | Root or Administrator, one action at a time | `apt install`, restart a system service, edit `/etc`, install a driver, change the firewall | **Always `ask`**, never `auto`, never remembered (see 6.4) |

An owner can only tighten these defaults per host, per caller or per scope; no setting lowers L2 below "ask each time".

### 6.3 Grants (the credential)

A **grant** is a signed token minted by the target host (or an approval authority it trusts) after policy or a human says yes. It contains:

- `aud`: the target instance id (never valid elsewhere, P2);
- `sub`: the caller agent key and instance;
- `level` and **scope**: path prefixes, an allowed-operation list, optional parameter constraints (the JEA idea: structured, not text patterns);
- `not_before`, `not_after`, and optionally `max_uses`;
- `approver`: who or what decided, and how (`policy:<rule id>` or `human:<device>`);
- an id, for audit and revocation.

Defaults: L0 and L1 grants last at most an hour; L2 grants are single-use and last about five minutes. Grants are **attenuable but never widenable**: a caller may derive a narrower grant for a sub-agent it spawns (shorter life, smaller scope, fewer uses), verified from the chain; a sub-agent can never exceed its parent. Revocation is by expiry first, plus a host-local deny list checked on every request, plus the existing instance `revoked` status.

### 6.4 Approval

Policy modes per (caller, host, level, scope): `deny`, `ask`, `auto`.

**How `ask` works.**
1. The request arrives and passes the identity checks. It is held.
2. The host opens an approval window through the **host-process channel**, the same mechanism as the credential broker, not an in-page dialog, so no agent can click it (P5). The result is accepted only on the host channel secret.
3. The window shows: who asked (agent, instance, host name, verified or not), **the exact operation** (full argv, working directory, environment changes, target user, files and byte counts), the stated reason, the risk class, and what the grant will allow and for how long. For file edits it shows the diff.
4. Choices: **Approve once**, **Approve for this scope for N minutes** (L0 and L1 only), **Deny**, **Deny and block this caller for an hour**. There is **no "approve everything"**.
5. Counter-fatigue (ASI09): prompts are rate-limited per caller; repeated denials escalate to a block; an L2 prompt cannot be pre-dismissed or batched; and the window does not take keyboard focus while the user is typing, so a stray keypress cannot approve it (approval needs a deliberate click after a short delay with a visible countdown).

**Where the human is.** The approver must be able to be somewhere other than the target's console, because many hosts are headless or in a closet. Phase 1 uses the target's own window; later phases add an **approver device**: another of the owner's installs, or a phone, receives the same prompt over the account and answers with a signed approval (ideally a hardware-backed, user-presence key such as a platform authenticator). The approval is a signed statement binding the request hash, so a compromised relay cannot alter what was approved.

**Self-approval is impossible.** An agent cannot approve a request it made; an approver key is never an agent key.

### 6.5 Elevation (L2)

Elevation must not be a permanent root shell behind a socket.

- A **separate, minimal elevation helper** per OS, started only for an approved L2 action and exiting after it ("no resident root"). Windows: a UAC-launched helper from the signed host exe (as designed in `SPEC_LAN_FIREWALL_SETUP`). macOS: an SMAppService helper whose XPC requirement pins our signing identity. Linux desktop: polkit (per-action authorization); Linux headless: a one-time admin-installed `sudoers` drop-in that allows exactly one wrapper, `agentmux-elevate`.
- The helper **does not trust srv**. It verifies the grant token's signature and `aud`, its expiry and use count, and that the **argv it is asked to run equals the approved argv exactly**, then executes. A compromised srv or agent therefore cannot widen what was approved, and a stolen grant works only for that one command on that one host.
- **No shell strings.** L2 runs an `argv` array. A request to run a shell is itself an L2 operation shown as such, and the prompt says so plainly.
- The helper's own surface stays tiny: `run(argv, cwd, env, as_user)` plus file operations expressed as typed calls, with every path and argument validated. Install is a **one-time per-host enablement** by an administrator (the OS prompts once), consistent with the firewall spec's "one prompt" principle; until it is installed, L2 is simply unavailable and the caller is told so.
- Hosts under organizational management can disable L2 entirely by policy.

### 6.6 Operations (the API)

Typed operations, not a raw tunnel (nothing like port forwarding, because Session Manager's logging gap shows what a side path costs):

| Operation | Level | Notes |
|---|---|---|
| `host.info`, `proc.list`, `service.status`, `log.tail` | L0 | bounded output |
| `fs.read`, `fs.list`, `fs.stat` | L0 within scope | symlinks resolved and re-checked; no path outside scope |
| `exec` | L1 (L2 with `elevate`) | `argv` array, `cwd`, `env`, timeout, output cap, no shell by default |
| `pty` | L1 (L2 with `elevate`) | interactive session; recorded |
| `fs.write`, `fs.patch`, `fs.delete` | L1 within scope (L2 outside `$HOME`) | atomic where possible |

Every operation has limits: wall-clock, output bytes, concurrent sessions per caller and per host.

**MCP surface.** `Shell`, `PtyShell*` and a new file toolset gain an **optional `host`** argument. Absent means local, exactly as today. A remote target must be named explicitly each call, never inherited from a previous call or from conversation state, so a local command can never silently become remote (P8). Results carry the grant id and the host that ran them.

### 6.7 Audit and recording

- Every request, **allowed or denied**, is written on the **target** host before it runs: time, caller identity and verification result, grant id, operation, the parameter hash and the parameters themselves (with secrets masked), approver, outcome, exit code, byte counts.
- The log is append-only and **hash-chained**; the agent that acted cannot read-write it, and the file is not on a path any agent route serves. A caller can read only its own entries through a typed call.
- PTY sessions are recorded as transcripts, with secret masking applied **before** storage (recordings that capture plaintext secrets are their own leak). Who can view recordings is itself a controlled permission.
- The audit log is on by default and cannot be turned off for L1 and L2. A host may forward it to the account for off-host retention, so a compromised host cannot erase the only copy.
- The status bar shows when remote sessions are active on a host and by whom.

### 6.8 Prompt injection and the confused deputy

The new danger is specific to agents: a hostile message makes a trusted agent ask for something it should not.

- **A grant belongs to an agent identity, not to a conversation.** Text in a jekt can never create, extend or select a grant.
- Requests record their **provenance**: whether the agent's current turn was driven by a jekt whose trust was `network-claimed` or whose tier was `sensitive`. The approval window shows it ("this request followed a message from an unverified sender"), and policy can require `ask` regardless of an `auto` rule in that case.
- L2 never skips the human, whatever the provenance, and the human sees the exact command, which defeats most injected instructions (the injected text cannot change what the window displays).
- A tool result from a remote host is untrusted input like any other and is never treated as an instruction.
- **Kill switch (P9).** One control, in the status bar and as a CLI, revokes all grants, drops all sessions and turns remote access off for the host; it also works from an approver device.

### 6.9 Discovery and addressing

Targets are named by instance (hostname alias plus instance id), resolved from the existing LAN peer list and the account key directory. `DiscoverAgents` already reports both. A name that resolves to more than one instance is an error, never a guess.

### 6.10 Migration from the SSH network

The SSH network stays as **break-glass** until the acceptance in section 8 passes on every host type. There is a parity checklist (run a command, edit a file, attach a PTY, elevate, transfer a large file, work on a headless host) and an explicit step that removes the stored logins afterwards. Nothing about this spec requires the SSH credentials to be read or copied; removing them is the owner's decision.

### 6.11 Hops: things behind a host

A grant is bound to **one host** (`aud`), so access is not transitive: being allowed to act on host A never lets a caller act on host B through A.

- **Preferred: address each AgentMux host directly.** Anything that can run AgentMux (a bridged VM, a laptop, a server) is reachable on its own over LAN or WAN with its own grant. Nothing needs a hop.
- **A target that does not run AgentMux** (a hypervisor's command line, a router, a vendor tool) is reached by a typed `exec` on a host that can reach it, under that host's own grant. The hypervisor is the standard case: `vmrun` runs on the VMware host, not in the guest.
- **No generic proxy or port forwarding.** It is the side path that bypasses logging (AWS Session Manager does not log forwarded sessions) and the approval prompt.
- **A request that host A makes to host B on behalf of a caller** is a new request, signed by A, that carries the original caller as `via`. B sees both identities, records both, and applies the **narrower** of the two policies. Default policy on every host: deny any request that has a `via`. An owner can allow it for named pairs of hosts, and L2 is never allowed over a hop. This stops a compromised or prompt-injected agent on A from borrowing A's reach.

### 6.12 What this spec does not cover: the first install

Putting AgentMux on a machine that does not have it yet needs a human or an existing channel (the SSH network, as break-glass). Everything here starts once a host runs AgentMux and its owner has turned remote access on.

### 6.13 Worked example: the Windows test bed

The VMware host (gamerlove) runs AgentMux. The clean Windows VM on it runs its own AgentMux and is bridged, so it is a LAN peer.

1. The caller asks the host agent on gamerlove, with an L1 grant for the `vmrun` operations only (scoped to the test VM's folder: list, revert to a named snapshot, clone, start, stop; nothing else), to revert the VM to the clean snapshot and start it.
2. When the guest's AgentMux is up, the caller addresses it directly, as an ordinary host, for L0 and L1 work: installing the build under test, reading logs, running the first-run checks. No hop through gamerlove.
3. Every step is in the audit log of the host it ran on, with the grant id.

Until phases 1 and 2 ship, the same flow works only agent-to-agent: a jekt asks the gamerlove agent to do the step with its own tools. That is useful but unaudited, and it rests on that agent's judgment and the host's default permission mode, which is the gap this spec closes.

## 7. Threat model

| # | Threat | Control |
|---|---|---|
| T1 | An attacker on the LAN reads the `lan_key` and calls routes | The remote plane never uses it (6.1); separate listener, mutual key authentication |
| T2 | Forged or replayed request | Signed over audience, parameters hash, timestamp and nonce; nonce store; short clock window |
| T3 | A grant for host A used on host B | `aud` bound to the target; checked by the target and by the elevation helper |
| T4 | Parameters changed after approval | The approval signs the request hash; the helper compares argv byte for byte |
| T5 | A prompt-injected agent asks for something harmful | Provenance shown and enforced; L2 always asks with exact parameters; scope limits on L0 and L1 |
| T6 | An agent approves its own request | Approval only on the host channel; approver keys are never agent keys; no agent route reaches the window |
| T7 | Approval fatigue | Rate limits, escalating blocks, no "approve everything", L2 not batchable |
| T8 | Stolen instance key | Same residual as WAN verification: valid until revoked; mitigations are short grants, per-host policy, the account's `revoked` status, and that L2 needs a human anyway |
| T9 | Compromised target host | It can lie about its own audit; off-host forwarding keeps a second copy; grants are bound to it and do not move |
| T10 | Compromised relay | Sees ciphertext only; cannot alter approvals (signed over the request hash) |
| T11 | Log tampering or secret leakage via recordings | Hash chain; masking before storage; recordings access-controlled |
| T12 | Resource exhaustion | Concurrency, output and time limits per caller and per host |
| T13 | Path escape (symlinks, `..`, junctions, case folding) | Resolve then re-check against scope on every operation; typed file ops only |
| T14 | Cross-account access | Not in scope for phases 1 to 3; the account directory only ever vouches for keys of the same account |
| T15 | The elevation helper becomes a privilege-escalation tool | One action per launch, no resident root, verifies the grant itself, tiny typed surface, signed binary, caller verified every time |
| T16 | A compromised or prompt-injected host borrows its reach to act on other hosts (confused deputy through a hop) | Grants are bound to one host; a forwarded request carries `via`, is denied by default, takes the narrower policy, is recorded on both hosts, and is never allowed for L2 (6.11) |

## 8. Acceptance (tests that must pass before the SSH network is retired)

1. A request authenticated only by the `lan_key` is refused on every remote-access route.
2. A valid request replayed, sent to another host, or sent after expiry is refused and logged.
3. L0 works with no prompt; L1 prompts the first time and not again inside the granted window; L2 prompts every time.
4. After approval, a changed argument, working directory or environment is refused (T4), including by the elevation helper called directly.
5. An agent driving the UI (`UIClick`, `UIQuery`) cannot approve a request or read its approval secret.
6. A request following a message from an unverified sender shows that provenance, and policy `ask` overrides a matching `auto` rule.
7. A `revoked` instance, and an instance whose certificate chain fails, are refused (never prompted).
8. The kill switch ends active sessions within a second and every later request is refused.
9. The audit log contains every allowed and denied request, is not readable or writable through any agent route, and a tampered entry breaks the chain check.
10. PTY recordings contain no plaintext for secrets typed at masked prompts.
11. Headless Linux, a Windows host (no console user), and macOS each complete the L2 flow end to end with an approver device.
12. Output and time limits stop a runaway command and a flood of requests.
13. A request with a `via` is denied by default; when allowed for a named pair of hosts it shows both identities in the prompt and in both audit logs, uses the narrower policy, and is refused for L2.
14. The test-bed flow of 6.13 runs end to end: revert and start the VM through a `vmrun`-scoped grant on the VMware host, then work on the guest directly, with no way to run anything else on the VMware host through that grant.

## 9. Delivery plan

Phases ship separately, each usable and reviewable alone. The gates before phase 1 follow the multi-tier spec's own ordering: remote invocation is last, behind enforced per-agent authorization.

| Phase | Scope | Gated on |
|---|---|---|
| **0 Prerequisites** | LAN reachable on a fresh install (`SPEC_LAN_FIREWALL_SETUP`, in progress); cloud-side per-agent authorization confirmed; the audit store; a design decision on the encrypted channel | those landing |
| **1 Read-only (L0), LAN, same account** | Signed requests, the encrypted channel, grants, policy `deny`/`ask`/`auto`, the host `host.*`/`fs.read`/`log.tail` set, audit, kill switch | phase 0 |
| **2 Act as user (L1), LAN** | `exec`, `pty`, `fs.write`, the approval window, provenance, recordings | phase 1 |
| **3 WAN** | Outbound dial-out tunnel through the relay, end-to-end encrypted, with the same grants | phase 2 |
| **4 Elevation (L2)** | The per-OS helper, one platform at a time (Windows first, as the helper is already designed for the firewall work), one-time enablement | phase 2 |
| **5 Approver devices and delegation** | Remote approval on another device with user presence; attenuated grants for sub-agents | phases 2 and 4 |

## 10. Relationship to existing specs

- Completes the "remote invocation" part of `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md` and fixes its transport question: the remote plane is a separate, encrypted listener, not the jekt routes.
- Reuses, and does not change, the trust markers and tiers of the jekt specs: a jekt can still never carry an instruction that bypasses a human, and nothing here widens what a `TRUST` value means.
- Depends on `SPEC_LAN_FIREWALL_SETUP_2026_10_01.md` for reachability and reuses its elevated-helper design for Windows.
- Does not alter the local default permission mode (`bypass`); that is a separate decision, though the remote plane deliberately starts stricter than it.

## 11. Open questions (owner decisions)

1. **Default for an approved same-account instance.** My recommendation: L0 `auto`, L1 `ask` per scope, L2 always `ask`. Would you rather L1 be `auto` for your own hosts, and if so scoped to which paths?
2. **Is L2 over WAN allowed at all?** Recommendation: yes, only with an approver device that proves user presence, never on the target's own auto policy.
3. **Channel choice.** TLS 1.3 with pinned raw public keys (widest library support, well understood) or a Noise-based channel (simpler, key-only, newer to this codebase). Recommendation: TLS with key pinning.
4. **Trust root.** Pin each peer's key trust-on-first-use (what LAN does today), or let the account vouch for instances and require pinning only for off-account peers. Recommendation: the account directory for same-account hosts, TOFU with an explicit confirmation for anything else, since phases 1 to 3 do not cross accounts.
5. **Headless approval.** Is a phone approval acceptable, or must approval always be on a machine you own? This sets whether servers are in scope for L2.
6. **Retention and privacy.** How long to keep audit logs and recordings on a host, and whether forwarding them to the account is on by default.
7. **Organizational policy.** Does an administrator-managed machine get an enforceable "no remote access" switch that a user cannot flip?
8. **Hops.** Is `via` worth building at all? Recommendation: not in phases 1 to 3. Direct addressing plus typed `exec` on the host that can reach a non-AgentMux target covers the known cases, and the hop adds a confused-deputy surface for no current need.

## 12. Sources

- NIST SP 800-53 AC-6 and SP 800-207, as summarized in vendor guides: [miniOrange, NIST PAM](https://www.miniorange.com/blog/nist-privileged-access-management/); [AvePoint, JIT privileged access](https://www.avepoint.com/topics/just-in-time-privileged-access); [Syteca, JIT PAM](https://www.syteca.com/en/blog/just-in-time-approach-to-privileged-access-management). (Vendor sources; the controls themselves should be read in NIST's own publications.)
- Teleport: [Just-in-Time Access Requests](https://goteleport.com/docs/identity-governance/access-requests/), [Resource Access Requests](https://goteleport.com/docs/identity-governance/access-requests/resource-requests/).
- SSH certificates: [Smallstep step-ca production considerations](https://prof.infra.smallstep.com/docs/step-ca/certificate-authority-server-production), [LWN, Using certificates for SSH authentication](https://lwn.net/Articles/913971/).
- Tailscale SSH: [docs](https://tailscale.com/docs/features/tailscale-ssh), [announcement](https://tailscale.com/blog/tailscale-ssh).
- AWS: [Systems Manager Session Manager](https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager.html).
- Agent security: [OWASP AI Agent Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/AI_Agent_Security_Cheat_Sheet.html), [OWASP GenAI Q1 2026 exploit round-up](https://genai.owasp.org/2026/04/14/owasp-genai-exploit-round-up-report-q1-2026/), [Auth0, lessons from the OWASP Top 10 for Agentic Applications](https://auth0.com/blog/owasp-top-10-agentic-applications-lessons/).
- MCP: [Authorization specification (2025-11-25)](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization).
- Delegation: [IETF draft, Attenuating Authorization Tokens for Agentic Delegation Chains](https://datatracker.ietf.org/doc/html/draft-niyikiza-oauth-attenuating-agent-tokens-00), [Macaroons](https://www.researchgate.net/publication/269196979_Macaroons_Cookies_with_Contextual_Caveats_for_Decentralized_Authorization_in_the_Cloud), [Identity Management for Agentic AI](https://arxiv.org/pdf/2510.25819).
- JEA: [Microsoft, JEA security considerations](https://learn.microsoft.com/en-us/powershell/scripting/security/remoting/jea/security-considerations?view=powershell-7.6).
- Privileged helpers: [SwiftAuthorizationSample](https://github.com/trilemma-dev/SwiftAuthorizationSample), [Michael Tsai, checking XPC peers](https://mjtsai.com/blog/2019/09/02/privilegedhelpertools-and-checking-xpc-peers/), [Elastic, named pipe impersonation](https://www.elastic.co/guide/en/security/current/privilege-escalation-via-named-pipe-impersonation.html), [GitHub Blog, polkit privilege escalation](https://github.blog/security/vulnerability-research/privilege-escalation-polkit-root-on-linux-with-bug/), [polkit manual](https://www.freedesktop.org/software/polkit/docs/0.105/polkit.8.html).
