# SPEC: one live instance per agent — an agent identity is driven by at most one process, across host, LAN and WAN

**Date:** 2026-09-24
**Status:** active — Phase 1 (host tier: lease, admission, fencing) implemented; see §12 for what was built and what
was left out. Phases 2–5 not started. §10's decisions are taken at their recommended option (repo owner, 2026-09-25:
proceed to implementation without further sign-off). §11's open questions are all answered from logs and code.
**Author:** Agent3 (UID `fb3e692d-caf9-48e3-b20a-e659361aa057`)
**Trigger:** Repo owner, 2026-09-24: *"by mistake I opened up an instance of you in a 57.2 version running on the same
host … we want a best practice method to ensure that 2 agents can't open simultaneously, even across the 3 tiers."*
**Researched against:** `agentmuxai/agentmux` `main` @ `01100e9c9` (v0.57.3); `agentmuxai/agentmux-cloud` `main` @
`fb93159`. The two instances in §1 ran v0.57.0 and v0.57.2.
**Related:**
- `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md` — the same failure, found on 2026-07-29. It
  named "cross-process turn ownership" as follow-up work. A lease was built for it (§2.2) but never wired to interactive panes.
- `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` §4.2 — rung **R0 Live** ("a controller for this agent is already running
  elsewhere → don't resume") is this spec's requirement, and today it only holds inside one process.
- `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md` — the agent UID this spec keys on; Q4 (two live blocks, one UID).
- `SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md` — the delivery tiers.
- `SPEC_ORPHAN_RECONCILER_CROSS_PLATFORM_LIVENESS_2026_09_20.md` — process-liveness probing to reuse in §4.2.

Evidence labels: **[verified]** I read the code or ran the check myself · **[spec]** stated by another spec, not re-measured ·
**[inferred]** reasoning, not observed. Times are UTC.

**"The 3 tiers"** are read here as the muxbus delivery tiers a running agent can be reached on: **host** (this machine —
including a *different AgentMux instance/channel* on it, `DELIVERY=channel`), **LAN**, and **WAN** (cloud relay). The
in-process case (one srv, two panes) is the degenerate host case and is covered by the same rule. If the owner meant
another split, only §4's layering changes; the invariants in §3 do not.

---

## 1. What happened, and what would have happened

### 1.1 The incident [verified unless noted]

Sources: both instances' srv logs (`channels/local-main-b28b7a-27a8ae9b/versions/0.57.0/logs/agentmuxsrv-v0.57.0.log.2026-09-25`
— "OLD" — and `channels/local-agentx-window-tab-skip-gate-f73571-0bcccffb/versions/0.57.2/logs/agentmuxsrv-v0.57.2.log.2026-09-25`
— "NEW"), the two provider transcripts, and the shared files named below.

| Time (UTC) | Event |
|---|---|
| 2026-09-24 17:48:30 | OLD (v0.57.0, block `f0153577…`) starts provider session `4f6c35e6…` under account `5370808b…`. The shared registry records it: `registry/fb3e692d….json` `session_id = 4f6c35e6…`. |
| 2026-09-25 02:23:56 | OLD is mid-turn (`turn_active flip … active: true`) and keeps writing the record `agent:fb3e692d…:current` every 30 s. |
| **02:25:31** | NEW (v0.57.2, block `204dc49b…`) opens Agent3. It reads `session_id = 4f6c35e6…` from the shared registry and tries to **eager-resume the session OLD is driving** (`persistent controller has a prior session … session_id: 4f6c35e6…`). Declined only by the credential gate ("No account linked"). The same second, the shared definition `definitions/fb3e692d….json` is rewritten (`updated_at = 1790303131190`). |
| **02:25:44** | An account is linked in NEW (`60a8fde6…`, config dir inside NEW's channel). NEW spawns `claude --resume 4f6c35e6…` — **a second driver on OLD's live session** — and re-registers `Agent3` on muxbus. |
| 02:25:45 | Saved by accident: `No conversation found with session ID: 4f6c35e6…` — the transcript lives under OLD's config dir, not NEW's. NEW clears the id and starts fresh session `4ef45828…`. |
| 02:27:14 | Last line OLD's provider session wrote. |
| 02:28:11 | OLD's CLI process is force-killed (`persistent process kill requested … force: true`). |
| **02:29:13** | A jekt arrives for Agent3 at OLD; OLD tries to respawn and is refused: `identity.spawn.blocked: no credentials … (definition fb3e692d…) — account 60a8fde6… row not found`. The binding NEW wrote to the **host-global** identity store (`shared/identity-store.db`) points at an account OLD cannot resolve. OLD is dead from here on. |

Overlap: **02:25:31 → 02:28:11**, both instances live. During it, `DiscoverAgents` listed **Agent3 twice** — block
`204dc49b…` on NEW (`127.0.0.1:51160`) and block `f0153577…` under `cross_channel` (`127.0.0.1:61134`) — and
`ListConversations` showed both.

**Why nothing worse happened:** NEW's `--resume` of OLD's live session failed only because the two instances used different
provider config dirs. With the same account (the common case) two CLI processes would have been appending to one
conversation — the 2026-07-29 incident exactly. **Nothing checked for a live holder at any point.**

**Why OLD lost its auth:** NEW did not only start a second driver; before any check could run it **wrote shared state** —
the agent definition and the agent's account binding, both host-global — and that write broke the first instance. A guard
that only refuses the second *spawn* would not have prevented this (§3 I9).

### 1.2 What two live instances of one agent share

Both carry the same agent UID, `fb3e692d…` **[verified]**: NEW's `DiscoverAgents` reports `uid: fb3e692d…` for block
`204dc49b…`, OLD's log names `definition fb3e692d…` for block `f0153577…` (§1.1, 02:29:13), and the shared registry record
`registry/fb3e692d….json` has `definition_id = instance_id = fb3e692d…`. Everything keyed by UID or by the host-global
shared root is therefore
**one object with two writers**:

- the AgentMux record, zone `agent:<UID>:current` in `~/.agentmux/shared/agents/transcripts/filestore.db`;
- the registry entry and the agent definition (`~/.agentmux/shared/agents/{registry,definitions}`) — including the
  registry's `session_id`, which is how NEW found OLD's live session to resume (§1.1);
- the agent's **provider-account binding** in `~/.agentmux/shared/identity-store.db` — how NEW broke OLD (§1.1);
- the agent's working directory — the retro's worst finding: a second process ran `git checkout` in the same checkout the
  first was using;
- personal memory and the work queue (`claimed_by` is by name/UID);
- **jekt routing**: `uid_to_block` is one-to-one, so a second registration **evicts** the first **[spec]**
  (`SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED` Q4). The older instance silently stops receiving messages and does not know.

What differs is the provider session. If the second instance resumes the *same* provider session, two CLI processes append
to one JSONL — the exact 2026-07-29 incident, and what NEW attempted here. If it starts a *different* one (what NEW fell back
to), the agent has two divergent conversations on one identity, and the user sees the second one as "lost its history".

---

## 2. What exists today, and where it stops

### 2.1 In-process guard [verified]

`agent.open` serializes the check-then-create sequence with a per-agent async mutex (`AGENT_OPEN_LOCKS`,
`server/app_api/agent_open.rs:22`) and refuses to seed `--resume` if `agent_live_elsewhere` finds a controller in
`CONTROLLER_REGISTRY` (`:647`). Its own comment states the boundary: *"this only serializes calls handled by THIS process …
it can't see a genuinely different AgentMux instance/channel racing the same registry entry."* Both structures are
in-memory and per-process. **Reach: one srv.**

### 2.2 Cross-process lease — exists, but interactive panes never take it [verified]

`registry::LeaseStore` (`agentmux-srv/src/registry/leases.rs`) is a real cross-process lease built for the 2026-07-29 retro:
files `<registry_root>/leases/<instance_id>.{lease.json,lock}` under the **host-global** shared root; claim/renew/release
each run inside a per-key OS advisory lock (`flock` / `LockFileEx`); `RENEW_INTERVAL_MS = 5 000`, `LEASE_TTL_MS = 15 000`;
an owner is a per-process `boot_id`; expired leases are reclaimable; release by a non-owner is a no-op. It is well built.

But the only claimant is the **`subprocess` controller**, per turn (`subprocess/host_spawn.rs:81-125`, test
`spawn_turn_refuses_when_lease_held_by_another_process`). The **`persistent` controller — the one interactive agent panes
use — has no lease code at all** (`grep -i lease` over `backend/blockcontroller/persistent/` finds only unrelated "release"
names). Direct evidence: on this host right now `~/.agentmux/shared/agents/registry/leases/` **exists and is empty**, while
Agent2, Agent3, Clamk and AgentX are all live. Reach for interactive agents: **none**.

(`backend/blockcontroller/agent_lock.rs` is a different thing — a PtyShell lease that locks out *human* input while an agent
types into a terminal. It is not an instance guard.)

### 2.3 Gaps this leaves, by tier

| Tier | Same-agent second instance today | Guard |
|---|---|---|
| Same srv, second pane | prompted / refused via `agent_live_elsewhere` | in-process only |
| **Host, different instance/channel/version** (this incident) | both run; second may resume the first's session or start a parallel one; jekt routing goes to the last registrant | **none** |
| LAN | possible if the same definition exists on two peers; no cross-peer check | none |
| WAN | possible, and jekts are **raced**: the relay ignores `subscribe` messages ("accepted but ignored — routing is broadcast", `agentmux-cloud` `muxbus/server/src/index.ts` @ `fb93159`), broadcasts a zero-metadata wake to every socket of the account, and each sidecar pulls `GET /reactive/pending/:agent_id` **by lower-cased name**. Two live copies of one agent both pull; whichever acks first gets each message | none **[verified]** |

### 2.4 Version skew is part of the problem

The 0.57.0 instance predates any lease this spec adds and will ignore it. A design that only works when **both** sides
cooperate would not have helped here. §4.2 therefore includes a compatibility probe that needs nothing from the older
instance.

---

## 3. Requirements

**I1 — Single driver.** At most one process may drive an agent identity (start turns, accept injected input, be the jekt
target) at a time. Identity is the **agent UID** — not the display name, block, pane, provider session id, account or
working directory. (The name-keyed fallbacks are what the UID migration exists to remove.)

**I2 — Newcomer yields, visibly.** When a second instance tries to start, the default is that **it** is refused, with a
message naming the holder (host, channel, version, pane, since when). Never a silent second driver; never a silent steal.

**I3 — Bounded staleness.** A crashed, killed or suspended holder can never wedge the agent forever. Every claim has a TTL
and is renewed; a dead holder is reclaimable within a small, documented bound.

**I4 — Fencing.** A holder that lost its claim (paused past TTL, partitioned, reclaimed) must **stop driving** and must not
be able to write shared state afterwards. Detecting expiry only at claim time is not enough — a holder that wakes from
sleep believing it still owns the agent is the classic failure of leases without fencing.

**I5 — Explicit takeover.** Moving an agent from instance A to B (the upgrade case) is a deliberate, audited action that
gracefully stops A first. It is never performed by a message: a jekt or muxbus reply must not be able to authorize it
(the STOP rule of `SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md`; a takeover is exactly the kind of action a spoofed message would ask for).

**I6 — Observers are free.** Reading an agent's transcript or history (`SearchHistory`, `GetAgentTranscript`, a read-only
pane) never needs the claim and is never blocked by it.

**I7 — Tier-independent rule, tier-appropriate guarantee.** The same rule applies everywhere. What can be *guaranteed*
differs: on one host it can be **prevented** (shared filesystem, one clock). Across a LAN or the WAN there is no shared
disk; with a partition, no design can both prevent duplicates and stay available (CAP). There it is **prevent when
reachable, detect and yield when not** (§4.3, §4.4). This spec does not pretend otherwise.

**I8 — Unknown is not free, except where stated.** Failing to reach a holder-check must be a deliberate, logged, per-tier
policy (§10 D2), never an accident of a timeout.

**I9 — Admission before any shared write.** A newcomer must be admitted **before** it writes anything shared on the agent's
behalf — the registry's `session_id`, the definition, the provider-account binding, the muxbus registration, the record.
§1.1 shows why: the second instance broke the first by rebinding its account, not by spawning. Opening a pane for an agent
that is live elsewhere writes nothing shared at all until the user chooses observe or take over (§4.6).

---

## 4. Design

### 4.1 One claim per agent UID, at one choke point

- **Key:** the agent UID. `LeaseStore`'s existing key is already it: every call site passes `block.meta["agentId"]`
  (`subprocess/mod.rs:116-122`, `server/agent_handlers/input.rs:902-907`, `server/app_api/agent_io.rs:355-359`), and for a
  user agent that equals the definition id and the registry's `instance_id` — `fb3e692d…` for all three in Agent3's
  registry record **[verified]**. Template launches get a fresh UID per launch **[spec]**, so they correctly never collide.
- **Store:** extend `registry::LeaseStore` rather than write a second one. Add to `LeaseFile`: `epoch` (u64, incremented on
  **every ownership change**, never reused), `channel`, `version`, `hostname`, and the holder's `pid` **plus process start
  time** (PID alone is reusable). `boot_id` stays the owner id. All new fields are `serde(default)`, so a lease written by
  an older build still parses — version skew must never look like corruption. **The epoch lives in its own per-key
  counter file (`<uid>.epoch`), never deleted** — release deletes the lease file, so an epoch kept only there would restart
  after a clean release and let a stale writer's epoch be reused (Codex P2 on #3730).
- **Lifetime:** the claim is per **CLI process lifetime**, not per turn (the `subprocess` path's per-turn claim leaves
  the gaps between turns open). Taken right before the process is spawned; renewed every 5 s by a task **independent of
  turns**; released when that process exits. A respawn in the same pane keeps the lease it already holds instead of
  re-claiming. A persistent controller with no process holds nothing — it is not driving the agent.
- **One entry point:** `AgentAdmission::acquire(uid) -> Granted{epoch} | Denied{holder} | Unknown{reason}`. **Every** path
  that can start an agent calls it before spawning — eager resume, first spawn, resume retry, the queue's leftover respawn,
  picker "Continue", `agent.open`, cron/work-queue spawns, jekt-triggered spawns. Per-path checks are exactly how today's
  guard ended up in-process-only and racy (`agent_open.rs:3-21`); the durable-memory spec's single resolver (§4.2, rung R0)
  is the natural home. Each path's existing in-process check becomes a fast pre-check in front of it, not a replacement.
- **Order (I9):** `acquire` runs **before** the open/resync path writes the definition, the account binding, the registry
  `session_id` or the muxbus registration. On `Denied` none of those writes happen. In §1.1 all of them happened at 02:25:31,
  in the same second the pane opened.

### 4.2 Host tier — prevent (shared root, flock, one clock)

The shared root is host-global and already carries the cross-channel registry (`~/.agentmux/shared/agents/reactive`), so
this tier can be a real critical section, not a best effort.

1. `acquire(uid)` enters the per-UID critical section and reads the lease.
2. **Free / expired →** claim, `epoch` = next value of the counter. **Unreadable → held (fail closed)** until the file has
   not been rewritten for a full TTL: a parse failure proves nothing about the holder, and a live holder rewrites the file
   on every renewal (Codex P1 on #3730).
3. **Held, unexpired, by another `boot_id` →** `Denied{holder}`.
4. **Held, but the holder is provably dead** (same host: `pid` gone or start time differs) → reclaim **immediately**, do not
   wait 15 s. Only when the lease names **this** host — a holder on another host (a shared root on a network drive) is
   never presumed dead. (The orphan reconciler named in Related probes CEF windows, not pids; `sysinfo` is used instead.)
5. **Compatibility probe for lease-unaware holders (every version before this ships).** No lease file exists for them.
   Before granting, also read the host-global cross-channel registry (`~/.agentmux/shared/agents/reactive`, `AgentEntry`:
   `agent_id`, `local_url`, `block_id`, `pid`, `channel`) for every **other** channel's live srv (`pid` alive), and ask each
   one — `GET <local_url>/agentmux/reactive/agents` with that entry's `auth_key` — for its registrations. A registration
   whose **`uid`** equals this agent's ⇒ treat as holder (`Denied{holder, lease: none}`). **Never matched by name**: display
   names are not unique, so a same-named but different agent must not block this one (Codex P2 on #3730). Registrations
   carry `uid` since #3560, before v0.57.0; an instance too old to report one, or one that does not answer within 1.5 s,
   is skipped and logged — not evidence either way. In §1.1 OLD's entry was exactly this (channel `local-main-b28b7a-27a8ae9b`,
   `127.0.0.1:61134`, visible to NEW as `cross_channel` in `DiscoverAgents`). Remove the probe once the oldest supported
   version carries the lease. (The registry's `registration_nonce` cannot serve as an epoch here: it is a per-srv counter
   that restarts at 1 — `persistent/mod.rs:116` — and the frontend presence path writes `0`.)
6. **Filesystem guard.** `flock`/`LockFileEx` are unreliable on network and cloud-synced folders. The shared root is
   `~/.agentmux/shared` unless `AGENTMUX_SHARED_DIR` or `AGENTMUX_HOME_OVERRIDE` says otherwise
   (`registry/paths.rs:133-154`); Windows known-folder redirection moves Documents/Desktop, not the home root, so only an
   explicit override can put it on a network or synced drive **[verified]**. When an override is set, check the volume
   (Windows `GetDriveTypeW` ≠ `DRIVE_REMOTE`; Unix `statfs` not NFS/SMB/FUSE); if remote, log loudly and degrade to step 5
   only — never silently proceed as if locked.

### 4.3 Fencing — a lost claim must stop the holder (I4)

- **In Phase 1, not deferred** (Codex P1 on #3730: TTL reclaim without fencing lets a woken holder and the new one both
  drive). The controller **verifies** the lease before every turn starts — both send paths, a read under the key's lock —
  and the renewal task treats a renew that finds another owner as **lost**. Either way the CLI process is killed and the
  turn refused; a transient I/O error is not a loss.
- On loss it enters a **Superseded** state: no new turns, no injected input accepted, the pane shows *"Agent3 is now driven
  by v0.57.2 on this host"* with **[Open read-only]**, and the controller goes through the existing graceful shutdown
  (`SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN`). It does not race the new holder.
- **Fence at the shared resource, not only at the holder.** Appends to the record zone `agent:<UID>:current` carry the
  writer's epoch; the store rejects an append whose epoch is **older** than the current lease epoch. A holder that never
  noticed it lost the claim then still cannot corrupt the record. (Phase 2, §9.)

### 4.4 LAN tier — detect and yield (no shared disk)

LAN peers already discover each other and exchange agent directories and public keys (`backend/lan_discovery.rs`, the
LAN-tier signing spec). Extend that, do not build a coordinator:

- The peer advertisement gains `live_agents: [{uid, epoch, acquired_at, host_id}]`.
- `acquire(uid)` first completes the host tier, then asks reachable peers (short timeout, ~1.5 s, signed request) whether
  they hold that UID. A live holder ⇒ `Denied{holder}`.
- Two hosts can still start at the same moment. **Deterministic tie-break, evaluated by both sides on the next
  advertisement:** earlier `acquired_at` wins; equal ⇒ lower `host_id`. The loser fences itself (§4.3) within one
  advertisement interval. **Never compare clocks across hosts** — compare only `acquired_at` values that each host measured
  for itself, and use the tie-break on `host_id` when they are closer than the skew budget (10 s).
- **Unreachable peers are not evidence of absence.** Policy in D2: proceed, log, and rely on detect-and-yield when the
  peer comes back. The duration of any overlap is then bounded by the partition, not by this design.

### 4.5 WAN tier — the relay as coordinator

The cloud relay is the only component every WAN participant already talks to, so it is the only place a *preventing*
guarantee can live. Today it has **no per-agent state at all** (§2.3): `subscribe` is ignored, the wake is broadcast per
account, and pending jekts are pulled by name. So this is new relay surface, not a tweak to subscriptions:

- **`POST /agents/lease`** `{agent_uid, agent_name, host_id, boot_id}` → `200 {epoch, ttl_ms}` or `409 {held_by: {host_id,
  hostname, acquired_at, renewed_at}}`. Conditional write keyed by `(account, agent_uid)` — in DynamoDB a
  `ConditionExpression` on `holder = :me OR expires_at < :now`, which is the relay store's native compare-and-set.
  **`PUT` renews** (every 20 s, TTL 60 s — WAN-scale, not the host's 5/15 s); **`DELETE` releases**.
- **Pending pulls are fenced by it.** `GET /reactive/pending/:agent_id` and `POST /reactive/ack` from a sidecar that does
  not hold the agent's WAN lease return `409 not_holder` instead of messages. This is what removes the race in §2.3: a
  second copy cannot consume the first copy's jekts even if it ignores everything else.
- The lease is keyed by the **UID**; the name stays for the pending-queue path until that is re-keyed too.
- Scope: the account the relay already authenticates. Two different muxbus accounts running "the same" agent are two
  agents to the relay — correct, since they cannot see each other's queues either.
- **This is a change to `agentmux-cloud`** (`muxbus/server/src/index.ts` and its Lambda twin), outside this repo.
- Offline behaviour follows I7: if the relay is unreachable, the host/LAN tiers still govern, the agent is allowed to run
  (offline work must not be bricked), and the relay resolves on reconnect — the unexpired holder keeps it, the late claimant
  gets `409`, yields and fences (§4.3).
- **After any overlap, the record forks.** Both branches were appended to `agent:<UID>:current`. The ledger records a
  `fork` event at the point of divergence rather than merging silently, so history shows that two conversations existed.

### 4.6 Takeover and the upgrade case (I5)

The incident was an upgrade in disguise: an older version was still open when a newer one launched. The newcomer, on
`Denied{holder}`, offers:

- **[Open read-only]** — an observer pane (I6); no claim needed.
- **[Stop it and take over]** — human-confirmed in the UI. The newcomer sends a *fence request* to the holder's `srv_url`;
  the holder finishes its current tool call, saves state and releases; the newcomer claims with `epoch + 1`. If the holder
  does not answer within TTL, the claim is simply reclaimable (§4.2 step 2) and the newcomer's UI says so.
- **[Cancel].**

A takeover request is accepted only from the local UI or from a `channel-verified` / `host-verified` /
`lan-verified` caller **and** with a human confirmation recorded; it is never actionable from a jekt body. A version that
predates the fence request cannot be stopped remotely; for it, takeover degrades to "wait for it to close" or a
human-confirmed kill of the pid named in the probe result.

---

## 5. Edge cases

| Case | Behaviour |
|---|---|
| Holder crashes / `kill -9` | same-host pid probe ⇒ reclaimed at once; otherwise expires in ≤ 15 s |
| srv restarts (new `boot_id`) | old boot's lease is dead by the pid probe ⇒ reclaimed; the restarted srv re-claims for its restored panes |
| Laptop sleeps past TTL, wakes | on wake the holder's next pre-turn check finds the claim lost or re-owned ⇒ **Superseded**, no turn is started (§4.3) |
| Two panes, same agent, one srv | same rule: one driver; the second is an observer or refused — replaces today's prompt-only behaviour |
| Template agents | fresh UID per launch **[spec]** ⇒ never collide, correct: each launch is a new agent |
| Sub-agents inside one session | not instances — same process tree, same claim |
| Terminal panes, drones | no agent UID ⇒ outside this spec (declared-outside set of the identity spec) |
| Clock skew | host tier: one clock. LAN/WAN: never compared across hosts (§4.4) |
| Same real account, two channels | irrelevant — the key is the UID, not the provider login |
| Lease directory on a synced/network folder | detected ⇒ degrade to the compat probe, loud log (§4.2 step 6) |

---

## 6. Observability

- Counters, per srv, on the same endpoint family as `GET /agentmux/identity/fallbacks`:
  `agent_admission.{granted, denied, reclaimed_expired, reclaimed_dead, unknown}`, `agent_admission.fenced`,
  `agent_admission.takeover`, `agent_admission.compat_probe_hit`.
- `DiscoverAgents` and `ListConversations` gain `holder` / `epoch` per agent; a duplicate becomes visible as data, not as
  something one has to notice in two separate lists (§1.1 needed both tools and a human to see it).
- A pane badge for **Superseded** and for **observer**.
- Every denial and takeover is one structured log line naming both parties.

---

## 7. Tests (each written to fail first)

1. Two `LeaseStore`s on one temp root: A holds ⇒ B `Denied`; A dropped without release ⇒ B granted after TTL; **epoch is
   strictly increasing across all transitions**.
2. Pid-probe reclaim: holder's pid dead ⇒ immediate reclaim; pid reused with a different start time ⇒ still reclaimed.
3. Compat probe: a live registry entry with **no** lease file ⇒ `Denied`, `lease: none` — the incident.
4. Fencing: holder past TTL ⇒ next pre-turn check yields **Superseded** and starts no turn; a stale-epoch append to the
   record is rejected (Phase 2).
5. **Every** spawn path goes through `acquire` — a test enumerating paths and failing when one is added without it (the
   opposite of today's per-path checks).
6. Two real srv processes, two channels, one shared root, one **fresh test agent** ⇒ second is refused; `kill -9` the first ⇒
   second claims within the bound.
7. LAN: two srv instances on loopback with static peers ⇒ simultaneous start, exactly one survives the tie-break within
   one advertisement interval. WAN: relay contract tests live in `agentmux-cloud`.
8. Takeover cannot be triggered by a jekt body of any trust tier.
9. **I9:** opening an agent that is held elsewhere leaves the definition file, the account binding, the registry
   `session_id` and the muxbus registration **byte-identical** — the §1.1 failure as a test.
10. WAN (in `agentmux-cloud`): second `POST /agents/lease` ⇒ `409`; a non-holder's pending pull ⇒ `409 not_holder`, and
    the message is still delivered to the holder.

Live checks use a `task dev` build and a **fresh, never-used test agent** — never a live agent's session (the retro's own
lesson, and the way that incident began).

---

## 8. What this spec does not do

- It does not merge two diverged conversations; it records the fork (§4.5).
- It does not make the LAN/WAN tiers linearizable. It bounds the damage and makes it visible.
- It does not change how jekts route between *different* agents.
- It does not decide history retention or the recall design — see `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` and
  `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md`.

---

## 9. Delivery order (one PR each; each ends with a live check on a fresh test agent)

0. **Now, no code:** treat "start a second instance of a live agent" as unsafe — the retro's rule; open a different agent,
   or a fresh test agent, for anything experimental.
1. **Host tier — prevent, with fencing.** `agent_admission::acquire` keyed by UID over the extended `LeaseStore`; wire the
   `persistent` controller (claim at spawn, independent renewal, release on exit); admission before any shared write (I9);
   pid-probe reclaim; compat probe for lease-unaware holders; pre-turn verify and kill-on-loss; refusal message naming the
   holder. This alone closes today's incident. **Implemented — §12.**
2. **Takeover UX.** Superseded state in the pane, fence request, the takeover dialog, observer panes; the §6 counters.
3. **Record fencing.** Epoch on record appends; store rejects stale epochs; `fork` ledger event.
4. **LAN — detect and yield.** Advertisement field, peer query, tie-break.
5. **WAN — relay coordinator.** UID-keyed lease endpoints and fenced pending pulls in `agentmux-cloud`; the client side
   here.

Phases 4 and 5 can proceed independently of 2–3. Phase 1 is small: the lease, the locking and the tests exist; what is
missing is the wiring and the key.

---

## 10. Decisions (taken at the recommended option — see Status)

- **D1. Default on conflict:** the **newcomer is refused**; takeover only by explicit human action. *Recommend yes.* The
  alternative (newest wins) would have stopped the healthy 0.57.0 instance mid-turn on a mistaken launch.
- **D2. Unreachable coordinator (LAN peer / relay):** fail **open** with a loud log and rely on detect-and-yield. The host
  tier always fails **closed** (its store is a local file). *Recommend yes* — the alternative bricks offline work.
- **D3. Key by UID only.** No name fallback in admission. Legacy panes without a UID are minted a row first (identity spec
  D1). *Recommend yes.*
- **D4. Keep TTL 15 s / renew 5 s.** They tolerate two missed renewals and the pid probe covers the common crash case.
  *Recommend yes;* shorten only if takeover feels slow.
- **D5. Observer mode for a refused second pane** instead of a plain error. *Recommend yes.*
- **D6. Compatibility probe** for lease-unaware holders, removed once the oldest supported version has the lease.
  *Recommend yes* — without it the first release does not protect against the very version pair in §1.
- **D7. Takeover needs a human confirmation and is never jekt-drivable.** *Recommend yes.*

---

## 11. Open questions from the first draft — all answered

| # | Question | Answer **[verified]** | Where it changed the design |
|---|---|---|---|
| 1 | Is `LeaseStore`'s `instance_id` the agent UID? | Yes for user agents: `block.meta["agentId"]` at every call site = definition id = registry `instance_id` | §4.1 key — reuse as is |
| 2 | Why did the second instance start fresh? | It tried `--resume` of the **live** first instance's session (from the shared registry); failed only on a config-dir mismatch | §1.1; the compat probe (§4.2 step 5) is required, not optional |
| 3 | Why did the first instance lose its auth? | The second rebound the agent's account in the host-global identity store to an account the first cannot resolve | new **I9**, §4.1 order |
| 4 | Relay semantics for a duplicate agent? | `subscribe` ignored; broadcast wake; name-keyed pull ⇒ jekts raced | §2.3, §4.5 rewritten, pending pulls fenced |
| 5 | Is `registration_nonce` a usable epoch? | No — per-srv counter restarting at 1, `0` from the presence path | §4.2 step 5 note; `epoch` is new |
| 6 | Can the shared root sit on a synced folder? | Only via an explicit env override | §4.2 step 6 — check the volume only then |

Still open, deliberately left to implementation: the exact list of spawn paths for test §7.5 (enumerated when Phase 1
routes them), and whether the 0.57.0 srv's "row not found" came from reading a per-channel account table or a schema gap —
irrelevant to the fix, which is I9.

---

## 12. Phase 1 as built

**Code.**
- `agentmux-srv/src/registry/leases.rs`: `claim_as` with `ClaimantInfo`, the epoch counter, unreadable-is-held, the dead
  and reused-pid reclaim, `verify`, `live_holder_other_than`, and a block-aware `release` (a superseded block of the same
  process must not drop the lease a newer block holds).
- `agentmux-srv/src/backend/agent_admission.rs` (new):
  - `acquire` returns a `HeldAgentLease`, which has its own renewal task, `verify`, and release on drop. It fails closed on
    I/O after one retry (D2).
  - `check_before_spawn` is the early read-only check: the lease plus the compat probe.
  - `denied_message` is the refusal text.
- `persistent/`:
  - `with_agent_lease_store`, used by the one place persistent controllers are built (`blockcontroller/mod.rs`).
  - `acquire_agent_lease` in `spawn_process`, just before `cmd.spawn()`. All four spawn paths pass through that point.
  - Release in both current-generation exit arms.
  - `fence_check` at the top of `send_message` and `send_user_message_with_policy`.
  - The early check in `try_eager_resume`, before the spawn claim and the credential gate.
- Early checks elsewhere:
  - `run_agent_turn`, before `build_persistent_spawn_env`. A refusal takes the gate's error path, so it is persisted and
    shown in the pane.
  - `agent.open`, right after its per-agent lock and before any write; the error is `AGENT_LIVE_ELSEWHERE: …`.
  - The frontend presence registration (`POST /agentmux/reactive/register`). It answers 409 and registers nothing, because
    delivery prefers the freshest cross-instance entry, so registering a refused pane would pull the live instance's
    jekts. This is a fourth I9 write that §1.1 had not listed.

**Tests.**
- `registry::leases`: 22 tests. Among them: fail-closed on an unreadable file, reclaim of a stale unreadable file, old
  format honoured, epochs across release and across TTL reclaim, dead pid, reused pid, another host, `verify`, and the
  superseded-block release.
- `backend::agent_admission`: 9 tests. Among them: refusal naming the holder, a takeover reported by the renewal task
  through `on_lost`, and the probe matching by UID only.
- `persistent::tests::single_live_instance`: 5 tests. Refused before spawn; lease held until the process exits; a pane that
  lost its lease starts no turn and is killed; no UID takes no lease; the early check.
- `agent_open::single_live_instance_tests`: the I9 test (§7.9). It passes with the check and fails when the check is
  removed.

**Not in Phase 1.**
- §4.2 step 6, the volume check for an overridden shared root.
- The §6 counters. Phase 1 has structured logs only: `agent_admission.{granted,denied,fenced,unknown,compat_probe_hit}`.
- The Superseded, observer and takeover UI. A refusal and a fenced turn surface as the spawn-gate error text in the pane.
- A spawn env without an agent UID takes no lease and is logged: tokenless and legacy panes (D3's minting is outside this
  spec).
- The subprocess controller keeps its per-turn claim on the same key. It now also sees persistent holders, which is
  intended.
