# SPEC: agent identity is carried, never derived

**Date:** 2026-09-23
**Status:** active — M0 shipped in #3543 (2026-09-23); M1a (mint and
carry the UID and token into the process) in #3548; M1b (UID columns on the
work queue and cron, dual-written) in #3550. M2 implemented in #3560 from the §4.4 design (revision 4.1). M3 in #3563.
M4–M5 not started.
Redesign of `SPEC_CANONICAL_AGENT_ID_MIGRATION_2026_09_21.md` after its Phase
2 was implemented and proven unable to fix the defect it targeted. Supersedes
that spec's §6 phase plan; its §2 inventory and §5 WAN analysis remain valid
and are cited rather than restated.
**Trigger:** Repo owner, after Phase 2's failure: *"redesign the entire
solution... the goal is agent id unique... do the most robust solution."*
**Evidence base:** five review rounds on #3520 (four P1s, one P0), a data check
across 47 live databases, and direct measurement of `resolve_agent_id`'s real
behaviour.
**Revision 3 (2026-09-23).** Three adversarial review passes so far. Revision 1
had four false empirical claims (§0); revision 2 added two more and got §4.1
wrong in the opposite direction from revision 1. §2's principle has survived
every pass unchanged; the mechanism has been rewritten each time. Read §0 and
§12 before trusting any specific claim here — the failure mode of this document
has consistently been confident prose about unexamined code.

---

## 0. What revision 1 got wrong

Recorded rather than quietly edited, because three of the four errors share one
cause — **asserting what the code does without running it** — which is the same
failure that produced the predecessor spec's unworkable Phase 2.

### 0.1 The frontend's UID is known-stale (was §4's central premise)

Revision 1 said the frontend *"already holds the UID"* via
`block.meta.agentInstanceId` and *"has been carrying the right value all
along"*. The storage layer explicitly refuses that field
(`backend/storage/agents.rs:2210-2212`):

> We deliberately do NOT consult `block.meta.agentInstanceId`: codex P1 on PR
> #1114 round 3 surfaced that pane reuse can leave a stale one behind.

There is a regression test pinning it. Pane reuse is exactly the respawn case
this spec must handle, so revision 1 proposed keying the registry on a value
already known to go stale in the case it most needed to be right.

**Consequence:** the frontend is not an identity source, and §4 no longer
treats it as one.

### 0.2 `AGENTMUX_AGENT_ID` has two different meanings today

Revision 1's table asserted it is the slug. It depends on the launch path:

- `server/agent_handlers/input.rs:341-353` — the **display name**.
- `frontend/app/view/agent/agent-model.ts:545` — the **slug**.

One variable, two meanings, two launch paths. `AGENTMUX_AGENT_SLUG` also
already exists in `pane_env.rs:74` and revision 1 never reconciled it. Any plan
that "keeps its present meaning" is describing a meaning that does not exist.

**Consequence:** reconciling this is now phase M0 — nothing may depend on that
variable until it means one thing.

### 0.3 `agent_credentials.rs` is cloud machinery, not local

Revision 1 proposed minting a per-agent token at spawn *"reusing
`agent_credentials.rs`"*. That module is per-agent **Cognito M2M** against
agentmux-cloud: it provisions via `POST /agents/provision` using the human's
own PKCE token, and its callers **fall back to the shared `MUXBUS_TOKEN`
whenever it returns `None`, by design** (module doc, `muxbus/agent_credentials.rs:1-23`).

Building on it would make pane launch depend on cloud reachability and a prior
human login, and the documented fallback preserves precisely the shared-key
spoofability §6 exists to remove.

**Consequence:** §6 mints locally and shares nothing with that module.

### 0.4 The performance claim was backwards

Revision 1 claimed hot paths *"lose a database read"*. They have none to lose:
delivery today is a pure in-memory `HashMap` hit
(`backend/reactive/handler.rs`), and the only store read on that path was the
one **#3520 itself introduced**. Registration is separately not free —
`registry.rs:444-449` documents a synchronous SQLite read serialized on the
store's single `Mutex<Connection>` — and carrying identity does not remove it.

**Consequence:** §7 now claims only that this design adds no reads to hot
paths, which is true and sufficient.

### 0.5 Also corrected

- **Slug suffixing must stay.** Revision 1 proposed deleting it. The
  collision-resolved slug is load-bearing for the agent's working directory
  (`agent-model.ts:807-814`) and its git identity (`input.rs:366`); removing it
  would have two same-named agents share a cwd. §3 keeps it.
- **Identity confirmers compare display names** (`handler.rs:534-560`, fed by
  `set_agent_id(agent_name)` at `input.rs:820`). Re-keying the registry without
  migrating them in the same change rejects every message as an identity
  mismatch. §4 now sequences them together.
- **Registry files are name-keyed and world-readable** (`reactive/registry.rs:459-473`
  writes `auth_key` with no `0o600`; only the per-instance path sets a mode).
  Jekt signing keys are name-derived too. §6 covers both.
- **§5 ignored every non-interactive ingress** — Slack, Telegram, Discord,
  WhatsApp, muxbus relay, cron, work queue. "Ask the human" is not available at
  a webhook at 03:00. §5.4 answers it.

---

## 1. The actual diagnosis

The predecessor spec framed this as *"the slug is used as a primary key in ~12
places; migrate those places to `db_agents.id`."* That framing is what produced
a Phase 2 that could not work.

The real defect is architectural:

> **Identity is reconstructed, at every boundary, from a lossy, mutable,
> non-unique display name.**

Five subsystems each independently re-derive "which agent is this?" from a
string a human chose and can change. They disagree, so the same bug has been
found five times (#2901, #2428, #3480, #3500, #3520).

### 1.1 Reconstruction is impossible in principle

This is the finding that killed Phase 2, and it is worth stating precisely
because it constrains every possible design.

`derive_slug` lowercases and collapses non-alphanumerics. Measured against the
real resolver:

```
resolve_agent_id("agenty")  = Ok("def-a")
resolve_agent_id("AgentY")  = Err(unknown agent)
resolve_agent_id("AGENTY")  = Err(unknown agent)
```

The registry is keyed by *display name* (`input.rs` passes
`block.meta["agentName"]`), so in production a resolver like this essentially
never resolves. Normalizing the input first is worse, not better:

```
Agent B (name "AGENTY") -> slug "agenty-2"
Agent C (name "AgentY") -> slug "agenty-3"
derive_slug(either)      -> "agenty" -> resolves to def-a   <- a third agent
```

`agent_def_insert` suffix-resolves, so two colliding names get `-2`/`-3` while
bare `agenty` belongs to someone else. Normalizing silently misroutes both onto
an unrelated agent.

**`derive_slug` is lossy, and that lossiness *is* the collision.** Any resolver
whose only input is the display name either fails to resolve or misroutes. The
information needed to separate the two agents is not in its argument. No
implementation care changes this.

### 1.2 The identity already exists

Measured across every `db_agents` row on one host:

| Rows | `id` shape |
|---|---|
| `is_template = 1` (184) | `id == slug` — `claude`, `codex`, `gemini` |
| `is_template = 0` (18) | **UUID, without exception** |

Every agent that actually runs, registers and receives messages **already has a
globally-unique UUID**. Templates are prototypes, not agents; they are never
message targets.

So this is not a migration to a new identity. It is a migration to **carrying
the identity that already exists** instead of throwing it away and guessing it
back later.

## 2. Principle

> **Mint once. Carry everywhere. Resolve only for humans.**

Three rules, in priority order:

1. **No internal code path ever converts a name to an identity.** Identity
   enters the system where it is known — at spawn and at registration — and is
   passed along. A function that takes a name and returns an identity exists at
   exactly one layer (§5), and no internal caller may use it.
2. **Identity is proven, not asserted.** At any trust boundary, identity is
   established by a secret the server itself issued, never by a name the
   request supplies (§6). Stated as "comes from the connection" in earlier
   revisions, which is imprecise: this is loopback HTTP, where there is no
   per-connection peer identity to read — a header is still an in-request
   assertion. What makes it trustworthy is that only the server and the agent
   it spawned know the value.
3. **Names stay human.** MCP arguments, audit logs, the UI and agent-facing
   docs keep readable names throughout (§5.3). This spec makes nothing uglier
   for a human or a model.

## 3. Canonical identity

`db_agents.id` — already unique (`TEXT PRIMARY KEY`), already a UUID for every
non-template row, already used correctly for CRUD and credential injection.

Called the **UID** throughout this spec, to keep it unambiguous against the
existing overloaded term "agent id".

**The slug stops being an identity — but its suffixing stays.** Revision 1
proposed deleting `agent_def_insert`'s `-2`/`-3` resolution on the grounds that
it only faked uniqueness for a field that should not have carried it. That
reasoning is right about identity and wrong about consequences: the
collision-resolved slug is load-bearing elsewhere. It names the agent's working
directory (`agent-model.ts:807-814`) and seeds its git identity
(`input.rs:366`), so removing it would put two same-named agents in one
worktree committing as one author.

The slug therefore keeps producing distinct *filesystem and display* values,
while no longer being consulted for *identity*. Those are separable concerns
and conflating them is what made the predecessor spec plausible.

`derive_slug` survives for one purpose only: matching in the disambiguation UI
(§5.2).

## 4. Carrying it

### 4.1 The server registers the agent — neither the frontend nor the agent asserts identity

This section has now been wrong twice, in opposite directions, and the
correction is worth stating as a rule rather than a patch.

- **Revision 1** had the *frontend* assert the UID. Wrong: its
  `block.meta.agentInstanceId` is documented as going stale on pane reuse
  (§0.1), and it races the row's creation.
- **Revision 2** had the *agent* assert it, holding a token. Also wrong: a
  PTY/shell agent is a bare terminal. It has no `agentmux-mcp`, no HTTP client,
  and its only outbound channel is bytes the frontend parses — so a token given
  to it would land in the pane's persisted scrollback. Today such agents are
  jekt-registered server-side from the block's `cmd:env`
  (`blockcontroller/shell/lifecycle.rs:269-280`), precisely because they cannot
  call back.

**The party that knows the identity authoritatively is the server.** It created
the `db_agents` row, it minted the UID, and it set the process environment. It
does not need to be told by anyone.

So registration stays server-side and becomes UID-keyed at the existing call
sites — `persistent.rs:3368` (inside `spawn_process`, before any output),
`agent_handlers/input.rs` (turn start), and `shell/lifecycle.rs:818-822` for
shell panes. The frontend's `/reactive/register` becomes a presence signal
keyed by `block_id`; the agent asserts nothing.

This also preserves a property revision 2 would have destroyed. Registration
currently *precedes* the process: `bootstrap.rs:2074-2100` exists to serve
registered-but-not-yet-spawned persistent controllers, a state every persistent
agent occupies after an srv restart, and
`REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03` records
what breaks without it. An agent that can only register once it is running
cannot be registered before it is running — revision 2 would have regressed
that permanently, and §9.2's counters would have reported success.

**Rule:** identity is asserted by whoever minted it. Everyone else is told.

### 4.2 What moves, and what must move together

| Boundary | Today | Becomes |
|---|---|---|
| Spawn env | `AGENTMUX_AGENT_ID` (two meanings, §0.2) | `AGENTMUX_AGENT_UID` + `AGENTMUX_AGENT_TOKEN`; name/slug split explicitly |
| Registration | frontend POSTs a display name | server registers from the row it created (§4.1); the frontend's POST is a presence signal keyed by `block_id`; no token involved (M4 is separate) |
| Registry keys | lowercased display name | UID |
| **Identity confirmers** | compare display names (`handler.rs:534-560`) | compare UIDs — **same change, or delivery breaks** |
| **Alias map** | stable display name | survives as the *stable* name binding, resolving to a block and through it to a UID when one is bound (§4.3, §4.4.2) |
| Work queue, cron | slug | UID, captured at authoring time (§5.4) |
| muxbus / WAN | slug | UID (§8) |

The two bolded rows are why revision 1's §4 would have failed in production:
re-keying `agent_to_block` while `agent_identity_confirmer` still compares
display names makes every delivery an `"identity mismatch"`. They are one
atomic change, not three phases.

### 4.3 The alias map survives — it resolves names this system cannot reach

Revision 2 deleted `alias_to_block`, claiming it existed only because the
primary key was renameable, so a UID primary key makes it redundant. **That is
wrong**, and the reason is the most load-bearing constraint in this document.

`INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md` describes a jekt tagged
`<!-- agentmux:agent_id=agentg -->` — *"the standing tag, unchanged since
before this agent was renamed"* — failing to resolve because no live subscriber
answered to `agentg`. Those tags are embedded in **GitHub pull request bodies**.
They are immutable third-party data: not in this repo, not in any database this
project owns, not backfillable by any migration.

Every PR this project's agents open carries one. They will keep resolving
years from now or they will silently stop routing review notifications.

So the alias map is not a workaround for a renameable key. It is the one place
where a **frozen historical name** is mapped to a **current identity**, and a
UID primary key does not replace that — it is exactly what the alias should map
*to*.

Revision 2 also scheduled its deletion at M2 while keeping
`AGENTMUX_AGENT_ID` emitting names through M4 (§9.4), so agents would have gone
on minting PR tags for two phases after deleting their only resolver.

**Corrected:** the alias map stays: aliases resolve to *blocks*, and through the block to a UID when one is bound (a block may have none — §4.4.2). New
tags should embed the UID going forward, but the old ones must keep working
indefinitely. This also falsifies revision 2's §5.4 claim that non-interactive
ingress "never holds a name": the muxbus relay resolving a PR-body tag holds
exactly that, and must go through the alias map rather than §5's resolver.

### 4.4 M2 design — revision 4.1, measured against `main` after M1

This is the adversarial pass on §4.1 the retro asked for before M2 is
built. Every "measured" claim below was read from the code at the cited
site; nothing in this section is inferred from an earlier section.

**Revision 4.1.** Revision 4.0 was reviewed by an adversarial pass of its
own (recorded in §4.4.6): one P0, six P1s, six P2s — the same ratio as
every earlier revision, from a design that had already been "measured".
The corrections are folded in below and marked **[4.1]**.

#### 4.4.1 What registers today, and with what

Four real registration sites (three further `register_agent` callers are
tests):

| Site | Key registered | Where the key comes from | UID available? |
|---|---|---|---|
| `blockcontroller/persistent/spawn.rs` (spawn) | `AGENTMUX_AGENT_ID` from the spawn env, also parked as the alias | env, built by `build_persistent_spawn_env` | **when the row exists** — `AGENTMUX_AGENT_UID` is in that env since M1a, and removed by `carry_agent_uid_env` when the row is not found (see Q1) |
| `agent_handlers/input.rs` Register-tail (every `Register` turn) | `block.meta["agentName"]` | block meta | **yes** — the same turn already resolved the row for M0/M1a; the UID is in hand, no second read |
| `server/reactive.rs::handle_reactive_register` (frontend presence) | `req.agent_id` = `agentName` meta (`agent-view.tsx handleAgentIdChange`), **or [4.1]** an id the *agent process itself* emitted over OSC 16162 (`termosc.ts:275` calls the same `handleAgentIdChange`) — an agent assertion relayed by the frontend | the frontend | **not trusted** (§0.1, and §4.1's "the agent asserts nothing"); resolved server-side by `block_id` — one store read this path did not have before, which **must run on `spawn_blocking`** like every other store read in an async handler (`eager_resume.rs`'s note on incident #1782) |
| `shell/lifecycle.rs` (PTY panes) | `cmd:env AGENTMUX_AGENT_ID` | user configuration | **no row, no UID** — name-only, counted |

Two things the four sites share, measured: every one of them keys on a
*name*, and `register_agent_with_nonce` (`handler.rs:222-281`) evicts both
the previous holder of that name (`:237-240`) and the previous name of that
block (`:243-247`). That eviction is the collision bug: two agents, one
name, last writer wins. **[4.1]** It is also, today, the *only* garbage
collector the in-memory registry has: there is no TTL sweep
(`update_last_seen` is `#[allow(dead_code)]`, `handler.rs:387`; the
`cleanup_stale` calls in `bootstrap.rs:1555-1566` are for the Tier-2
*files*). A dead block's registration is removed either by an explicit
unregister or by the next same-name registration. Any design that stops
evicting by name must replace that collector (Q9).

Delivery inputs are all names today: MCP `SendMessage.to`, cron `target`,
the cloud re-inject (`cloud_subscriber.rs:945`), PR-tag stable IDs via the
alias map, `FleetBroadcast`, `record_supervisor_decision`,
`GetAgentTranscript`. None carries a UID yet. M3 moves resolution to the
MCP boundary; **M2 must keep every one of these working by name.** That
is a deliberate, transitional violation of §2 rule 1 ("no internal code
path ever converts a name to an identity"), stated here rather than
implied: through M2 the registry resolves names to blocks internally,
counted, until M3 moves it to the boundary.

#### 4.4.2 The registry after M2

Identity becomes the key; names become **typed** bindings.

```
uid_to_block:   HashMap<uid, block_id>          // the key, when known
block_to_uid:   HashMap<block_id, uid>
name_to_blocks: HashMap<lowercased name, Vec<block_id>>
block_names:    HashMap<block_id, NameBindings>
agent_info:     HashMap<block_id, AgentRegistration>     // keyed by block

struct NameBindings {
    display: String,        // exactly one; REPLACED on every re-register
    stable: Option<String>, // AGENTMUX_AGENT_ID parked at spawn; KEPT
}
```

**[4.1] Bindings are typed, because rename semantics depend on it.** The
`display` binding is the name a block was most recently registered under
(the Register-tail and the HTTP path supply it every time); re-registering
a block under a new display name **replaces** the old display binding, so
a retired name does not linger and make a later agent ambiguous. The
`stable` binding is the `AGENTMUX_AGENT_ID` parked at spawn — today's alias
— and is **kept** across display changes, which is what
`INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md` requires. `name_to_blocks`
indexes both kinds; a block therefore appears under at most two names.

`AgentRegistration` gains `uid: Option<String>`; **[4.1]** its existing
`agent_id` field is defined as the *display* binding, which is what its
consumers already treat it as (the Tier-2 heartbeat in `bootstrap.rs`,
discovery in `server/mod.rs`, `progress_watcher.rs`, muxspect). Nothing
that reads `agent_id` sees a change.

**Eviction is by identity, never by name.**

- Registering a UID on a new block evicts that UID's old block and all of
  its bindings — a respawn.
- Registering a name on a block never evicts another block holding the
  same name **unless** the two blocks share a UID, or **neither has one**.
  The no-UID case keeps today's name eviction exactly, and is counted
  under the registering site (Q12).
- A block with no UID that later registers *with* one is upgraded in
  place: same block, bindings kept, UID attached.
- **[4.1] A UID, once bound to a block, is sticky.** A later registration
  of the same block *without* a UID — the frontend presence path when its
  store read fails, or a shell-style re-register — keeps the UID it has.
  Without this, a transient store miss would drop a live block out of
  `uid_to_block`: UID-addressed delivery would fail and a later respawn of
  the same UID could no longer find and evict it — a ghost. This is
  consistent with §10 constraint 2 read precisely: the UID is state
  *registered for this block*, not state inferred by comparing against a
  previous value.

The alias map does not disappear (§4.3): the stable name is one of the two
typed bindings, and PR-tag names resolve through `name_to_blocks` like any
other.

#### 4.4.3 Delivery after M2

`inject_message_inner` resolves `target_agent` in this order:

1. **It is a UID** — `uid_to_block` hit → deliver. New: §5.2's "address one
   directly by uid" becomes possible. `validate_agent_id`
   (`sanitize.rs:124-131`) already accepts UUID-shaped strings.
2. **It is a name** — `name_to_blocks`, **[4.1] filtered to live blocks**
   (Q9): candidates whose block no longer has a controller are swept
   (unregistered, counted `registry.swept_dead`) before the count is
   taken;
   - one live block → deliver, counted `delivery.resolved_by_name` (the
     normal case until M3, so informational, not an M5 gate);
   - several live blocks → **refuse, with the candidates** (uid, block,
     display name). This is the only place a collision is visible and
     therefore the only place it can be resolved (§5.2). Today: silent
     delivery to whichever registered last. **[4.1] The error string must
     not begin with `agent not found`** — `handle_reactive_inject`
     (`reactive.rs:1031-1035`) treats that prefix as "forward to Tier
     2/2b/LAN", and a local collision must not be forwarded to other
     instances. It begins `ambiguous agent name:`.
3. **Neither** → `agent not found`, unchanged, so forwarding keeps its
   trigger.

**Identity confirmers move in the same change** (§4.2). `Controller` gains
`stable_agent_uid()`, set once at spawn from the same env
`stable_agent_id()` is set from (`persistent/spawn.rs:389-397`).

**[4.1] The UID confirmer is a separate check, not an extension of the
existing one — this was revision 4.0's P0.** Today's check
(`handler.rs:540-556`) runs whenever *either* name confirmer returns
`Some` and requires the target to match one of them. A persistent agent
after an srv restart is registered but not spawned (§4.1,
`bootstrap.rs:2074-2100`): its live name is set without a spawn
(`reactive.rs:1510-1511`), so `agent_id()` is `Some("AgentY")` while
`stable_agent_uid()` is `None`. A UID-addressed delivery run through the
existing check would compare the UID against the *name*, fail as an
identity mismatch at `handler.rs:584`, and never reach the
start-on-delivery fall-through — exactly the registered-but-unspawned
state revision 2 was faulted for breaking. Rule: **when the target resolved
by UID, only `stable_agent_uid()` is consulted; `None` is "unverifiable"
and delivery proceeds** (the same "absence of proof is not a red flag"
philosophy the name check already documents); when the target resolved by
name, the two name confirmers apply exactly as today.

`record_supervisor_decision` (`handler.rs:981`) resolves its target through
the same three steps — it is the site #3520 converted the others around and
missed (§10 constraint 4).

**[4.1] Name lookups outside delivery.** Revision 4.0 defined resolution
only for `inject_message_inner`. Seven other callers go from a name to a
registration or block, and under name bindings a name can hold several:

| Caller | What it needs | Rule |
|---|---|---|
| `ui_handlers.rs:106-114` — UI-automation authorization scope (own-pane close) | the caller's own block | **fail closed**: an ambiguous name authorizes nothing. The caller has a block id (its `AGENTMUX_BLOCKID`); the authz path should key on it |
| `reactive.rs:1827` — `GetAgentTranscript` by name | one block | one live → serve; several → refuse with candidates, same shape as delivery |
| `reactive.rs:1432` `/reactive/agent?id=`, `:112` sender echo, `:2389` badge, `service/misc.rs:121` cron `ListActive` | a registration | `get_agent(name)` returns `Some` only when exactly one live block holds it; otherwise `None`, counted `lookup.ambiguous_name` |
| `input.rs:1201-1215` — `AgentStop` | tear down one block | by block: `unregister_block` clears **all** its bindings and the cloud subscription for **each** name, not just the one `agent_id_for_block` returns |

#### 4.4.4 Adversarial questions, answered from the code

**Q1. "Registration precedes the process" (§4.1) — so can the UID be known
at registration?** Measured: `launchAgentDefinition` writes the block meta
(including `agent:sessionid` for a continuation, `agent-model.ts:763`),
then calls `ControllerResyncCommand` (`:775`), and only then
`CreateAgentInstanceCommand` (`:790`, best-effort). **[4.1] Revision 4.0
said the spawn itself waits for the first message, citing a frontend
comment. False for continuations:** `start()` eager-resumes whenever
`agent:sessionid` is present (`persistent/mod.rs:1060`), and
`eager_resume.rs` builds the env and calls `spawn_process`, which
registers — all before the instance row exists. For a template-derived
launch, `instance_get_active_for_block` (`agents.rs:2218-2244`) then keys
on `meta.agentId` = the *template* id, excluded by `is_template = 0`, and
falls back to `last_block_id`, which is not this block yet → no row →
`carry_agent_uid_env` removes `AGENTMUX_AGENT_UID` → the spawn registers
**name-only**. (For a user agent, `meta.agentId` *is* the `db_agents` row,
so the UID resolves even pre-row.) The design already covered it: no-UID
registration is legal and counted, and the first turn's Register-tail
upgrades the block in place. What changes is the claim, not the mechanism
— and the test in §4.4.5 now exercises this exact order.

**Q2. Quick-launch panes.** No row (M0's finding), so name-only forever.
**[4.1]** They register through the same `spawn.rs` site as every other
persistent agent, so the server cannot count them separately from a
transient row miss — the counter is `registration.no_uid.spawn` for both
(Q12), and its floor will not be zero until quick-launch gets a row or is
declared outside jekt addressing. That is the counter doing its job (§9.2).

**Q3. Pane reuse and the stale `agentInstanceId`.** Not consulted.
`instance_get_active_for_block` resolves `block.meta.agentId` first
(`agents.rs:2213-2253`), which the new launch overwrites; `agentInstanceId`
is not read anywhere in `agentmux-srv/src`.

**Q4. Two live blocks with the same UID.** **[4.1]** `instance_create`
(`agents.rs:1614-1620`) folds a launch into the definition's own row when
`def.is_seeded == 0` (user agents) or when a continuation names a live
parent; a **template** launch gets a fresh row. So two panes share a UID
for user agents and continuations, not for template relaunches.
`uid_to_block` is one-to-one, so the second registration evicts the first
— what the name-keyed registry does today for the same case. Not a
regression; recorded so nobody discovers it as one.

**Q5. The spawn-path deadlock** (`try_register_agent_with_nonce`, incident
2026-09-07). Unchanged: the UID comes from the env the spawn already holds
(`spawn.rs:387` reads `config.env_vars` before `:419`), so nothing new is
looked up under the handler's lock. The liveness probe in step 2 (Q9) is a
controller-registry read, the same kind the confirmers already make under
the lock; it is not a store read.

**Q6. §7's performance claim, corrected.** Delivery: one more `HashMap`
probe plus a controller-liveness probe per candidate only when a name is
ambiguous — no store read; the claim holds. Registration: the Register-tail
reuses the row M0/M1a already resolved (no new read); `spawn.rs` reads the
env (no read); **`handle_reactive_register` gains one store read** it did
not have, on the frontend presence path, on `spawn_blocking`. §7's
"registration unchanged" was true of three sites and false of the fourth.

**Q7. Teardown must not fail closed** (§10 constraint 3). HTTP unregister
carries only `agent_id` (`reactive.rs:1698-1701`) and **[4.1] answers
`{"success": true}` unconditionally (`:1743`)** — so revision 4.0's
"several → unregister none, log at warn" would have been precisely the
silent-success no-op the constraint forbids. Rule: `UnregisterRequest`
gains an optional `block_id`; with it, teardown is by block. Without it:
one live block → unregister it; several → unregister **none** and answer
**HTTP 409 with the candidates**, never `success: true`; counted
`unregister.ambiguous_without_block`. Both frontend callers are updated:
`termagent.ts` has the block id (`registeredAgentsByBlock`); the Warden
host manager's deregister works from agent rows that carry one.

**Q8. What M2 leaves name-keyed on purpose.** The Tier 2/2b file registry
(`reactive/registry.rs`, moves with the credential path in M4 per §6.3), the
cloud subscriber's `add_agent`/`remove_agent` (WAN is §8), and the subagent
watcher. Each keeps receiving the display name it receives today. **[4.1]
One consequence is stated rather than hidden:** the Tier-2 heartbeat
(`bootstrap.rs:1598-1610`) writes one name-keyed file entry per
registration, so two same-named live blocks — legal for the first time
after M2 — would take turns overwriting one file entry, and
*cross-instance* forwarding to that name would reach whichever wrote last.
Local delivery refuses the ambiguity; remote delivery cannot see it until
the files are UID-keyed in M4. Today the situation cannot arise because
only one such block can be registered.

**Q9. [4.1] Ghost registrations — the collector §4.4.1 says must be
replaced.** Today a pane that dies without its frontend `onCleanup`
(`agent-view.tsx:666`) firing, or a block removed by any path other than
`AgentStop`, leaves a registration that the *next same-name registration*
silently reclaims. Under eviction-by-identity nothing reclaims it, and a
relaunch of the same agent (a fresh row for a template, Q4) would be
ambiguous with its own corpse until an srv restart — every by-name input
refused. Rule: **liveness is checked at resolution, and dead candidates
are swept there.** The handler gets a `block_liveness` probe (set in
`bootstrap.rs` beside the confirmers, backed by
`blockcontroller::get_controller`); when a name resolves to more than one
block, candidates with no controller are unregistered on the spot and
counted `registry.swept_dead`. Ambiguity is decided among the live. A
periodic sweep is deliberately *not* added: lazy sweeping at the one
moment it matters is cheaper and cannot race a registration in progress.

**Q10. [4.1] UID stickiness** — see the last eviction rule in §4.4.2 and
its §10 note.

**Q11. [4.1] Contradictions this revision resolves elsewhere in the
spec.** §4.2's table rows for "Registration" and "Alias map" reflected
revision 2 and are corrected to match §4.1/§4.3 (registration stays
server-side, no token; the alias survives as the stable binding). §4.3's
"re-keyed so aliases resolve to UIDs" is made precise: aliases resolve to
*blocks*, and through the block to a UID when one is bound — necessarily,
since a block may have no UID. §2 rule 1's transitional violation is
stated in §4.4.1.

**Q12. [4.1] Counter sites that exist.** `registration.no_uid.spawn`,
`registration.no_uid.register_tail`, `registration.no_uid.http_register`,
`registration.no_uid.shell_pane`; `delivery.resolved_by_name`;
`delivery.ambiguous`; `lookup.ambiguous_name`;
`unregister.ambiguous_without_block`; `registry.swept_dead`. Reported
through the same `uid_fallback_counts()` snapshot M1b introduced.

#### 4.4.5 The fixture every test uses

Two agents whose names collide under `derive_slug` — `"AgentY"` and
`"AGENTY"` — with distinct UIDs, on two blocks. Asserted, positive case
first:

- both stay registered (today: the second evicts the first);
- delivery by name refuses with both candidates listed, and the error does
  not begin with `agent not found`;
- delivery by either UID reaches its own block and passes the UID
  confirmer;
- **[4.1]** delivery by UID to a block whose `stable_agent_uid()` is
  `None` (registered, not spawned) proceeds — the P0 case;
- a respawn of one UID on a new block evicts only that UID's old block;
- a rename replaces the display binding and keeps the stable binding; the
  old display name no longer resolves;
- a block with no UID keeps today's eviction semantics and is counted;
- a no-UID registration followed by a UID registration of the same block
  upgrades in place, and **[4.1]** a UID registration followed by a no-UID
  one keeps the UID;
- **[4.1]** a name held by a live block and a dead one resolves to the live
  block, and the dead registration is gone afterwards;
- unregister by block clears every binding; unregister by ambiguous name
  without a block clears nothing and is an error, not a success.

Mutation checks on: eviction-by-name (must fail the "both stay registered"
test only), the ambiguity refusal, the in-place upgrade, UID stickiness,
and the UID-confirmer `None` rule.

#### 4.4.6 Revision history of this section

- **4.0** — written after M1 landed, from the code. Reviewed by ReAgent in
  13 seconds as docs-only: "LGTM". Reviewed by an adversarial pass with
  the code open: **one P0** (the UID confirmer rejected every
  registered-but-unspawned agent), **six P1s** (eager resume spawns before
  the row for continuations; untyped bindings left rename undefined; seven
  name lookups outside delivery undefined; a UID could be dropped by a
  later UID-less registration; removing name eviction removed the only
  garbage collector; ambiguous unregister answered success), **six P2s**.
  Every P0/P1 was a claim about code the author had read — the retro's
  pattern, again.
- **4.1** — this text. A bare LGTM on a design is not a review (retro §4);
  the adversarial pass is the review.

## 5. The one place names are interpreted

Resolution is not plumbing. **It is a user-facing disambiguation problem**, and
treating it as plumbing is the root error of the predecessor spec.

### 5.1 One entry point

```rust
pub fn resolve_name_to_uid(store: &Store, typed: &str) -> NameResolution;

pub enum NameResolution {
    /// Exactly one agent matches.
    One(String),
    /// Several do. Carries enough to ask the human which.
    Ambiguous(Vec<AgentCandidate>),
    None,
}
```

Callable **only** from surfaces where a human or a model typed the name: the
MCP tool arguments, the CLI, the UI. Enforced by placement (its own module,
`pub(crate)` to those callers) and by a grep gate in CI (§9.3) asserting no
other module references it.

### 5.2 Ambiguity is surfaced, never guessed

`Ambiguous` is not an error to be collapsed into "not found". It carries the
candidates, and the caller renders them:

```
Two agents match "AgentY":
  • AgentY   (uid 4f3c…a91, block "term-3", started 14:02)
  • AGENTY   (uid 9b2e…7d4, block "term-7", started 09:41)
Address one directly by uid, or rename one.
```

A model receiving this can retry unambiguously; a human can act on it. Compare
today's behaviour: silently deliver to whichever registered last.

This is the only place in the system where a collision is *visible*, which is
correct — it is the only place one can be resolved.

### 5.3 Names still work

For the overwhelmingly common case — one agent, one name — `SendMessage(to:
"AgentY")` behaves exactly as now. Nothing about this spec pushes UUIDs into
anything a human or model reads or types. The difference is what happens in the
uncommon case: a question instead of a wrong answer.

### 5.4 Non-interactive ingress never resolves at all

Revision 1 scoped resolution to "MCP, CLI, UI" and ignored Slack, Telegram,
Discord, WhatsApp, the muxbus relay, cron and the work queue. None of those has
a human to ask at fire time.

**They do not resolve, because they never hold a name.** Resolution happens
once, at *authoring* time, when a human or model was present:

- Creating a cron job resolves the typed name **then** and stores the UID. A
  job firing at 03:00 addresses a UID, so ambiguity is impossible by then.
- Enqueuing work resolves at enqueue and stores the UID.
- A chat integration binds a channel to a UID at configuration time, not per
  message.

This turns "what does a webhook do with `Ambiguous`?" into a question that
cannot arise, rather than one needing a tie-break rule. Where a UID can still
fail to resolve — the agent was deleted — the correct behaviour is an explicit,
logged failure, never a fallback to name matching. Falling back would
reintroduce §1.1 at the least observable point in the system.

**Corollary:** stored references are UIDs, so renaming an agent cannot break a
cron job or a queued task. Today it silently does.

## 6. Identity is proven, not asserted

Closes #3501 and the env-inheritance retro's recommendation 4 together.

`AGENTMUX_AUTH_KEY` is instance-wide and inherited by every pane
(`pane_env.rs`'s own keep-set comment says so). It authenticates *"some caller
on this instance"* and can never answer *"which agent"*, so every authorization
decision reading `agent_id` from a request body is unenforceable by
construction.

### 6.1 Minted locally, not in the cloud

Per §0.3, this shares nothing with `muxbus/agent_credentials.rs`. That module
is cloud Cognito M2M, requires the human's PKCE token, and deliberately falls
back to the shared token — all three disqualifying.

The token here is a local secret the server generates when it creates the
`db_agents` row, stored server-side against the UID and injected at spawn. No
network, no login, no fallback. If it is missing the request is refused, which
is the entire point: a fallback to a shared credential is the property being
removed.

### 6.2 What it fixes

- The server maps token → UID on every request. **That** is the caller's
  identity.
- `agent_id` in a body becomes untrusted input: it may name a *target*, never
  assert the *actor*.
- `check_s1` compares connection-UID against target-UID. #3508 already made it
  resolve both sides at one point of comparison; this removes the remaining
  assumption that `ctx.agent_id` was trustworthy in the first place.

### 6.3 Two holes revision 1 left open

**Tokens must not be readable by other agents — and file permissions cannot
deliver that.**

Revision 2 first claimed `registry.rs:459-473` wrote `auth_key` with no mode
set. That was wrong: the write goes through `write_entry_file`, which sets
`opts.mode(0o600)` (`registry.rs:702`) and documents it. Corrected here rather
than deleted, because checking it surfaced the sharper problem.

`0o600` restricts by **user**, and every agent on a host runs as the *same*
user. That function's own doc comment says as much — *"same auth_key exposure
boundary as the per-channel registry: same-user trust boundary, not
cross-user"*. Against the threat this spec cares about, agent B reading agent
A's credential, the mode bits do nothing at all.

Two consequences:

- A per-agent token **cannot** be protected by dropping it in a file, whatever
  its permissions. It is held in server memory against the UID and injected
  into the agent's process environment at spawn; process isolation, not
  filesystem permissions, is the boundary (§6.4).
- The entry file is keyed by **name** (`shared_agent_dir(shared_dir,
  agent_id)`), so two agents sharing a name already share one file and one
  `auth_key` entry — a live instance of this spec's root defect in the
  credential path specifically. Entries move to UID-keyed paths regardless of
  what they contain.

The same applies to jekt signing keys, which are name-derived today
(`jekt_public_key_for(agent_id)`): a name-derived signing key is one that
anyone sharing that name can forge, and no file mode changes that.

**Expiry must not strand a running agent**, and neither must an srv restart.
A token that expires mid-task turns a healthy agent silent — the failure mode
#3520's first P1 showed is easy to create and hard to notice. So tokens are
long-lived for the process lifetime and revoked on deletion, not renewed:
renewal is a liveness dependency on the very path being secured, and the UID is
never reused.

Revision 2 then said the token lives "in server memory", which reintroduces the
same failure at a different seam: **any srv restart — crash, auto-update,
`task dev` — would leave every running agent holding a token nothing can
verify**, with renewal forbidden by the rule above.

`AGENTMUX_AUTH_KEY` survives restarts only because the *launcher* owns it and
re-supplies it (`agentmux-launcher/src/srv_spawner.rs`), and a per-agent token
has no such external owner. It must therefore be **durable server-side state
keyed by UID**, restored on boot like any other agent state.

That is not in tension with "a token cannot be protected by a file" above: the
objection there is to a *name-keyed, agent-readable* file in shared space, not
to the server persisting its own secrets where only the server reads them. The
distinction the spec must hold is **who can read it**, not whether it touches
disk.

### 6.4 Bootstrapping

The token is injected at spawn by the server that created the row, so there is
no chicken-and-egg: the agent never authenticates in order to *obtain* it.

**Revision 2's claim that `PANE_ENV_KEEP` governs its confidentiality was
wrong.** `pane_env.rs`'s `keys_to_strip()` enumerates *srv's own* environment
and removes only **inherited** variables. A token set explicitly on the child
command is untouched by `sanitize_process_command`, and is therefore inherited
by every descendant of the agent — its shell tools, build commands, any
postinstall script. `PANE_ENV_KEEP` does not constrain it in either direction.

Two things follow, neither optional:

- Descendant inheritance must be handled deliberately, not assumed away. An
  agent's own subprocesses holding its credential is a smaller blast radius
  than today's instance-wide key — the retro's stated fallback position — but
  it is not zero and the spec must not claim it is.
- Delivery to the MCP process needs care: `backend/agent_config.rs` writes
  `mcpServers.agentmux.env` into a `.mcp.json` **inside the agent's working
  directory**, rewritten every launch. A token must not go there; that file is
  in a worktree a human may commit.

## 7. Performance

Revision 1 claimed hot paths *"lose a database read"*. §0.4: they have none to
lose. The honest claim is narrower and sufficient.

| Path | Today | This design |
|---|---|---|
| Message delivery | in-memory `HashMap` hit, no store read | **unchanged** — UID is the key, still one hit |
| Registration | a synchronous SQLite read on the store's single `Mutex<Connection>` (`registry.rs:444-449`) | unchanged; identity arrives with the request |
| Work claim | string compare on a slug | UID equality on an indexed column |
| Authz | resolve both sides per call (#3508) | one token→UID map hit, then equality |
| Name → UID | scattered, per call site | once, at authoring time only (§5.4) |

**The design adds no read to any hot path.** It removes the one #3520
introduced, which is a regression this spec must not repeat rather than a win
it can claim. The remaining registration read pre-exists and is out of scope —
noted so a future reader does not mistake it for something this design caused.

## 8. Cross-machine

The predecessor spec excluded WAN (§5 there) because *"`db_agents.id` is a
per-machine UUID with no cross-machine relationship."* That reasoning holds for
"the same logical agent on two hosts" — but that concept does not exist and
should not be invented. A UUID is globally unique by construction, so:

- WAN addresses by `(account_id, uid)`. No name is ever routed on.
- Two same-named agents on two machines are **two agents**. That is correct,
  not a limitation — they are different processes with different state.
- If a user genuinely wants two hosts' agents federated, that becomes an
  explicit link between two UIDs, never a name coincidence. Out of scope here,
  but this design makes it expressible; the current one cannot.

This removes the predecessor's permanent WAN exclusion: subsystem 6 joins the
migration rather than being carved out of it forever.

## 9. Migration

Every phase is independently revertible and separately observable. No phase
changes two things at once.

### 9.1 Phases

- **M0 — Make `AGENTMUX_AGENT_ID` mean one thing.** Per §0.2 it is the display
  name on one launch path and the slug on the other, and `AGENTMUX_AGENT_SLUG`
  already exists. Split explicitly into name and slug variables, both set on
  both paths, before anything depends on either. Pure disambiguation, no new
  concepts — and nothing below is trustworthy until it lands.

  **Shipped in #3543.** `build_persistent_spawn_env` now sets
  `AGENTMUX_AGENT_DISPLAY` from `block.meta["agentName"]` and
  `AGENTMUX_AGENT_SLUG` from the block's `db_agents` row on the server path;
  the frontend path already set both. Two things measured while doing it,
  recorded because §0.2 did not know them:
  - **There are three launch paths, not two.** The quick-launch path
    (`launchAgent` in `agent-model.ts`) carries a *provider key* as
    `agentId` and has no `db_agents` row at all, so it has no slug in the
    identity sense. M0 leaves `AGENTMUX_AGENT_SLUG` unset there rather than
    derive one from the name; M1 must decide whether such a pane gets a row
    (and so a UID) or is declared outside the identity system.
  - **Server knowledge wins over `cmd:env`** for these two variables, the
    reverse of the surrounding "user-provided values take precedence" rule:
    the `cmd:env` copy is a launch-time snapshot a rename leaves stale.
    Disagreement is logged (§9.2), not silently resolved.
  `AGENTMUX_AGENT_ID` itself was neither read nor written, per §9.4.
- **M1 — Mint and carry.** Server mints the UID and token at row creation and
  injects both at spawn; add UID columns alongside slug columns; dual-write.
  **No reader changes.** Revertible by ignoring the new fields.

  **Split into two PRs**, so each is independently revertible:

  **M1a — shipped in #3548.** `build_persistent_spawn_env` resolves the
  block's `db_agents` row once and sets `AGENTMUX_AGENT_UID` (= `id`) and
  `AGENTMUX_AGENT_TOKEN` (`storage/agent_tokens.rs`, `db_agent_tokens`,
  schema v37). Both are reserved, server-controlled values: overwritten
  unconditionally, removed when no row is known — a stale persisted token
  would be another agent's credential. Decisions §12 left open, now taken:
  - **Token lifetime.** Process-lifetime, durable server-side, revoked via
    `purge_agent_dependents` on delete, no rotation. Minted at *first spawn*
    rather than at row creation, because every existing row predates the
    table; `agent_token_ensure` is idempotent so the two are equivalent for
    rows created later.
  - **The UID is not minted here** — measured: `db_agents.id` is already a
    server-minted UUID (`instance.rs`, `core.rs` create handlers). "Mint"
    in this phase's title was already true; only "carry" was missing.
  - **Quick-launch panes** (M0's finding) still get no row, so no UID and
    no token. Left for M1b/M2 rather than decided silently.
  - **Measured constraints honoured:** the env sanitizer runs before the
    explicit overlay (`blockcontroller/core.rs`), so a nested dev
    instance's inherited token is stripped while the child's own survives;
    `.mcp.json` carries neither variable; the container exec spec does
    carry the token (same user boundary as the docker socket) — stated,
    not claimed zero, per §6.4.

  **M1b — shipped in #3550.** `db_work_queue.target_agent_uid` /
  `claimed_by_uid` (identity store v10) and `db_cron_jobs.target_uid`
  (shared store v11, mirrored in the identity store), dual-written and
  read by nothing. Two mechanisms, deliberately: the target is *resolved*
  once at enqueue/create through `agent_resolve::resolve_uid_for_dual_write`
  (§5.4 — authoring time, never fire time), and the claimer's UID is
  *carried* — `agentmux-mcp` sends `AGENTMUX_AGENT_UID` on `WorkClaim`,
  with resolution of `agent_id` only as a counted fallback. Measured and
  recorded rather than hidden:
  - **A display name does not resolve** (§1.1's exact-case slug tier), so
    a `target_agent` typed as `"AgentY"` leaves the column empty and is
    counted. M3 fixes this at the MCP boundary through §5's entry point;
    a normalising resolver would misroute (§1.1) and is not the answer.
  - **The queue is global, the resolver is per-channel**, so a target in
    another channel stays empty here.
  - **`db_cron_jobs` lives in the shared store**, not the identity store
    the queue uses; the handlers read the shared copy. Both copies got the
    column. Cron's store placement is a pre-existing unmigrated case
    (`SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` step 1b), not changed here.
  - **§9.2's counters exist now**: `agent_resolve::uid_fallback_counts()`,
    in-memory, keyed by call site. No endpoint yet — M2 should expose them
    together with the registry's own fallback counter.
  - Every path that clears `claimed_by` clears `claimed_by_uid` (§10
    constraint 2).
- **M2 — Registration and delivery, atomically.** Registration stays
  server-side (§4.1) and becomes UID-keyed; the registry, **both identity
  confirmers**, and the **alias map** move in the *same* change (§4.2).
  Splitting them rejects every delivery as an identity mismatch. Slug fallback
  retained **with a counter** (§9.2).

  Revision 2 had M2 deriving the UID from a token — which token verification
  does not exist until M4. That ordering was unshippable. Because §4.1 now
  keeps registration server-side, M2 needs no token at all and the dependency
  disappears rather than being scheduled around.

  **On "no phase changes two things at once":** M2 changes four. The rule is
  about independently *revertible* units, not line counts — these four share
  one key and reverting any one alone leaves the registry and its confirmers
  disagreeing, which is strictly worse than either state. Atomic here means
  smaller, not larger.

  **Shipped in #3560**, implemented from §4.4 revision 4.1 with no
  deviations from it: identity-keyed registry, typed name bindings,
  eviction by identity, ambiguity refused with candidates, liveness sweep
  at resolution, sticky UID, a separate UID confirmer whose `None` is
  unverifiable, HTTP 409 on ambiguous unregister, per-site counters. Three
  of the mutation checks §4.4.5 names were run: eviction-by-name (both
  guards removed) fails six fixture tests, UID-confirmer-None-as-mismatch
  and non-sticky-UID each fail exactly one. Left name-keyed as §4.4.4 Q8
  says: the Tier-2 file registry (M4), the cloud subscriber (§8), the
  subagent watcher.

  The implementation had its own adversarial pass (one P1 — a block
  re-registered under a *different* UID kept its old `uid_to_block` entry,
  which ReAgent found independently — and eight P2s, six fixed in the same
  PR). Two were measured, accepted and are recorded here rather than fixed:
  - **A persistent pane whose process exited but is still open counts as
    live** (`get_controller` still returns it; its registration survives
    because the exit-time `unregister_block_if_nonce` never matches once the
    Register-tail has zeroed the nonce). Relaunching the same template while
    that pane is open creates a second identified agent with the same
    display name, and by-name delivery is refused until the old pane is
    closed. Before M2 the relaunch silently evicted the old pane. This is
    §5.2 working as designed — a respawnable pane *is* addressable
    (start-on-delivery, §4.4.3) — but it is a user-visible change, and Q9's
    "ambiguous with its own corpse" wording overstated what the sweep
    covers: it covers blocks with **no controller**, not exited processes.
  - **A fifth spawn site §4.4.1 missed:** the App API `agent.send` path
    (`server/app_api/agent_io.rs`) builds its spawn env without
    `build_persistent_spawn_env`, so it carries no `AGENTMUX_AGENT_UID`
    and registers name-only under `registration.no_uid.spawn`. An M1a gap,
    not an M2 one; fixed as its own follow-up.

  **Design: §4.4 (revision 4.1).** Written after M1 landed, measured against
  the four real registration sites and every delivery input. Names become
  bindings, identity becomes the key, eviction is by identity, ambiguity is
  refused with candidates, and the no-UID paths keep today's semantics and
  are counted. §4.4.4 answers the questions revision 3 left open — in
  particular that a persistent controller can legally register *before* its
  row exists and be upgraded in place, and that §7's "registration
  unchanged" was false for the frontend presence path.
- **M3 — Work queue and cron.** Columns hold UIDs; one-time backfill of live
  rows. Resolution moves to the MCP boundary so tool arguments stay readable.

  **Shipped in #3563.** §5.1's entry point exists
  (`backend/name_resolution.rs::resolve_name_to_uid`), reachable only
  through `POST /agentmux/agents/resolve`, with
  `scripts/check-name-resolver-callers.sh` as §9.3's grep gate. `agentmux-mcp`
  resolves `WorkEnqueue.target_agent` and `CronCreate.to` at the boundary,
  carries the UID, and hands an ambiguous name back to the model with the
  candidates (§5.2). A cron job with a captured UID fires by UID; a
  UID-addressed work item is claimable by that identity under any display
  name. Decisions and deviations, recorded:
  - **No backfill — neither rewrite nor drain** (§12's open question,
    answered a third way). The queue and cron tables are global and carry
    no channel column, while a name can only be resolved against one
    channel's `db_agents`; a channel-scoped backfill would bind a row to
    whichever channel happened to run first and had a same-named agent —
    and once bound, the UID path delivers there even after a rename
    (ReAgent P1 on #3563; the first cut of this phase shipped exactly that
    migration and was reviewed out). Pre-M3 rows keep resolving by name
    through the name paths that stay until M5, counted, and age out as
    they complete; new rows carry a UID from the boundary. Nothing is
    guessed and nothing is lost.
  - **A UID-addressed item is claimable by that identity only** — the name
    branch of the claim predicate applies to rows with no UID, never as a
    second way into a UID-addressed one (ReAgent P1 on #3563: the first
    cut let an exact same-name claimer through, and its test had dodged
    that case with a different-case name). A claimer that registered
    name-only before its row existed is therefore locked out of its own
    UID-targeted work until it carries the UID; recorded, and the first
    Register-tail after its row exists resolves it.
  - **A fourth variant, `Unidentified`.** A live block with no `db_agents`
    row exists and is reachable by name; returning `None` would say it does
    not. The spec's three-variant enum did not account for M0's finding.
  - **The resolver lists, it does not pick** — which is why it *can* match
    display names case-insensitively where §1.1 showed a picking resolver
    must not: one match resolves, a collision is refused with candidates,
    nothing is misrouted.
  - **Name paths stay** for rows with no UID in the claim predicate and in
    the cron fire until M5, counted (`cron.fire_by_name`), so a name-only
    row or target still works.
- **M4 — Proven identity.** Per-agent token; authz reads connection identity;
  body `agent_id` demoted to untrusted. Closes #3501.
- **M5 — Remove the scaffolding.** Delete slug fallbacks, delete
  `agent_def_insert`'s suffix-resolution, demote `AGENTMUX_AGENT_ID` to
  `AGENTMUX_AGENT_NAME`. Only after §9.2 shows the fallbacks are cold.

### 9.2 The exit criterion is measured, not assumed

Each fallback increments a counter tagged with its call site. M5 proceeds only
when they read zero across a full release cycle.

This is the mechanism the predecessor lacked: it had no way to know whether a
phase had actually taken effect, which is precisely how Phase 2 could be
"complete" while being a no-op in production. **A phase that cannot be observed
to work has not been shown to work.**

**A counter that never reaches zero is itself the finding.** Review of revision
1 noted that if some launch path can register without a UID, M5 is unreachable
— or ships and strands those agents. That is the correct behaviour for this
gate, not a flaw in it: a non-zero counter names a path that still reconstructs
identity, and M5 must not proceed until that path is fixed or explicitly
declared unsupported. §4.1 removes the known instance (the frontend's racing,
stale UID); the counters exist to catch the ones not yet known.

### 9.3 Guardrails

- A CI grep gate: no module outside the human-boundary layer may reference
  `resolve_name_to_uid`. Rule 1 of §2 becomes mechanically enforced rather than
  a convention that erodes.
- A test fixture with two agents whose names collide, exercised by every phase.
  #3520 proved that without one, a test suite passes while the feature does
  nothing.

### 9.4 Compatibility

`AGENTMUX_AGENT_ID` keeps its present meaning and value through M4:
`gh-agent.sh`, `bash_wrap.rs`, the OSC prompt hook and agent-facing docs all
read it. It is renamed only in M5, with both set during a deprecation window.

## 10. Failure modes #3520 demonstrated

Revision 2 called these *"bugs in `main` today, independent of how keying is
redesigned"* and recommended landing them separately. **That was wrong, and it
is the same error §0 and §12 exist to warn about.**

Checked against `main`: the reactive handler has no resolver at all, and every
key site — registration (`handler.rs:234`), unregistration (`:311`), delivery
(`:482`) and `record_supervisor_decision` (`:990`) — uses
`agent_id.to_lowercase()`. They are mutually consistent. There is nothing to
salvage because nothing is broken there yet.

All four were defects **introduced by the Phase 2 attempt**: three in code that
existed only on that branch, and one site I converted the others around and
missed. They are recorded here not as work to do, but as **constraints any
implementation of §4 must satisfy** — each was found by review rather than by
tests, and each would otherwise be rediscovered:

1. **Do not put a store read on the lookup path.** Resolving per lookup means a
   transient store error makes a registered, healthy agent unreachable. §4.1
   avoids this by construction: identity arrives with the request, so there is
   nothing to look up.
2. **Derive state from what is registered, never from what was previously
   stored.** A binding computed by comparing against a prior value cannot tell
   "the same agent re-registered differently" from "a second agent appeared",
   and gets it permanently wrong.
3. **Teardown must not inherit delivery's fail-closed rule.** Refusing to guess
   a recipient is right for delivery; applied to `unregister` it silently
   no-ops while reporting success, leaking the registration.
4. **Convert every key site together, or none.** Missing one leaves it keyed
   differently from the rest, and delivery keeps working — so the failure is
   invisible in tests while the logic around it is wrong.

Constraint 4 is why §4.2 requires the registry, both identity confirmers and
the alias map to move atomically rather than in sequence.

## 11. What this spec is not

- **Not a rename of everything to UUIDs.** §5.3: humans and models keep names.
- **Not a new identity.** §1.2: the UUID already exists on every real agent.
- **Not incremental repair of the predecessor.** Its Phase 2 cannot be patched
  (§1.1); the phases here replace that plan rather than continuing it.

## 12. Confidence

**Revision 2 note.** Revision 1 was reviewed adversarially and four of its
load-bearing empirical claims were false (§0). Every claim below that begins
"measured" was re-run against `main` for this revision.

Revision 2 then repeated the mistake twice, which is worth recording precisely
because both happened *inside* the correction:

- §6.3's permissions claim was taken from a review and written in without being
  checked. It was false.
- §10 asserted four defects were "bugs in `main` today". Checked afterwards:
  `main` has no resolver, every key site agrees, nothing is broken. They were
  defects the Phase 2 *attempt* introduced.

**Reviewers are not a substitute for running the code** — including reviewers
right about everything else. And a claim that follows naturally from something
already verified is not thereby verified.

Revision 3 corrects §4.1 (wrong twice, in opposite directions), §4.3 (deleting
a map that resolves immutable third-party data), §6.3 (a token nothing can
verify after an srv restart), §6.4 (`PANE_ENV_KEEP` does not govern what it
was claimed to), §2's rule 2, and §9.1's unshippable phase order. Each came
from adversarial review; none from a test. The pattern in both the
predecessor spec and revision 1 is the same — a plausible statement about the
code, written without running it, load-bearing by the time anyone checks — so
treat any unmeasured assertion here as suspect rather than as merely
unverified.

- **Measured, not inferred:** §1.1's resolver behaviour, §1.2's id shapes
  across 202 rows, the frontend already holding `agentInstanceId`, and
  `agent_credentials.rs` existing. Each was run.
- **High:** that carrying beats deriving. It is the only option left once §1.1
  shows deriving is impossible.
- **Medium, deliberately unresolved here:** the per-agent token's lifetime and
  rotation (§6); whether M3's backfill should rewrite live work-queue rows or
  drain them; whether `AGENTMUX_AGENT_ID` should be repurposed in place or
  retired alongside a new name. Each is a decision for its own phase, flagged
  rather than guessed — the predecessor's habit of resolving such questions in
  prose and discovering the answer in production is what this document exists
  to end.
