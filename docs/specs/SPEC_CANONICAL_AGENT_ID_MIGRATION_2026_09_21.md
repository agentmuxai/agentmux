# SPEC: retire the agent slug as a lookup key — `db_agents.id` becomes canonical

**Date:** 2026-09-21
**Status:** active — repo-owner-approved direction, scope confirmed by a
full-codebase audit (this document). Shipping as sequential PRs, one per phase.
Phase 0 and Phase 4 are complete. Landed: the `instance_get_by_slug`
fail-closed fix (#3500), registry-guard test coverage (#3503), Phase 0's shared
resolver (#3504, which also subsumes Phase 1 as §6 defines it), Phase 4
(#3508), and Phase 0's §4 safety net (#3514).
Remaining: Phase 2 (needs the rollout decision §6 flags), Phase 3 and Phase 5
— both carry a recommendation NOT to proceed as written; see their §6 entries
before picking either up.
**§2.5 records a verification pass against `main` @ `059cc6e` (2026-09-22)
correcting §2/§4/§7, plus two later self-corrections (§2.5.2, §2.5.5) where
this document asserted system behaviour inferred from a single function
without tracing its callers or checking the data. Read §2.5 before planning
any phase.**
**Trigger:** Repo owner, after `SPEC_CROSS_CHANNEL_AGENT_HISTORY_
RESOLUTION_2026_09_21.md` traced a concrete bug to `AGENTMUX_AGENT_ID` (a
non-unique human-readable slug) being compared against `db_agent_identity_
links.agent_id` (which actually stores `db_agents.id`, a UUID): *"the
AGENTMUX_AGENT_ID is very old vestige... it sounds like errors originate
there a lot."* Decision, stated directly: *"lets get rid of agentmux_agent_id
... it should be the enforced-unique ID... it is painful, but it will allow
the app to scale much more easily."*
**Builds on:** `SPEC_CROSS_CHANNEL_AGENT_HISTORY_RESOLUTION_2026_09_21.md`
(Bug A there is the smallest instance of the pattern this spec fixes
system-wide).
**Related:** `SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md` (the trust
boundary `agent_slug()` already relies on — see §3), the Memory/Identity
subsystem's existing (inconsistent, triplicated) resolvers this spec
consolidates rather than replaces (§4).

---

## 1. Problem

`AGENTMUX_AGENT_ID` — a human-readable, model-and-human-chosen slug (e.g.
`"AgentY"`) — is used in roughly a dozen places across the codebase as if
it were a primary key: as a `HashMap` key, a SQL `WHERE` comparison value,
and an authorization-equality check. `db_agents.slug` (the column this value
ultimately traces back to) has **no `UNIQUE` constraint**
(`agentmux-srv/src/backend/storage/migrations.rs:618-640`). Nothing prevents
two agent definitions from sharing a slug today, and at least one existing
resolver (`Store::instance_get_by_slug`, see §4) already has to silently
pick "whichever matching row was updated most recently" when that happens,
rather than erroring — a second, smaller instance of the same class of bug.

`db_agents.id` **is** a properly enforced-unique `TEXT PRIMARY KEY`. It is
already used correctly as the canonical key for agent CRUD, credential
injection at spawn, and (after a prior fix, see §4) the Memory/Identity/
Preset App API family. It is simply not used consistently — most of the
runtime-messaging and work-tracking surfaces never adopted it.

## 2. Inventory (full audit — see below for methodology)

A full-codebase audit was run against `agentmux` @ `4ab7a9ea9` across
`agentmux-srv/`, `agentmux-mcp/`, `frontend/`, and `agentmux-bashwrap/`.
Every site reading `AGENTMUX_AGENT_ID` (or an `agent_id`/`agent`/
`target_agent`/`source_agent` parameter that traces back to it rather than
to `db_agents.id`) was categorized:

- **(a) genuine lookup/comparison key** — the risky pattern; a slug
  collision here causes real cross-talk or a false negative.
- **(b) display/logging/audit text only** — no risk, not in scope.
- **(c) spawn-time injection only** — where the value is born; not itself a
  lookup, but feeds every (a) site downstream.
- **(d) documentation/type-comment only, no live code** — not in scope.

### Category (a) sites, grouped by subsystem — this is the migration's scope

| # | Subsystem | Core files | Notes |
|---|---|---|---|
| 1 | **Reactive handler registry** (SendMessage, FleetBroadcast, GetAgentTranscript, SupervisorNudge, cron-fire, cloud-relay re-inject, block registration) | `agentmux-srv/src/backend/reactive/handler.rs` (`agent_to_block`, `agent_info`, `register_agent_with_nonce`, `handle_reactive_message`), `server/reactive.rs` (`handle_reactive_register`, `handle_reactive_transcript`), `agentmux-mcp/src/main.rs` (SendMessage, FleetBroadcast, GetAgentTranscript, SupervisorNudge dispatch arms), `muxbus/cloud_subscriber.rs` (re-inject) | The single largest, most-connected surface — an in-memory `HashMap<lowercased-slug, …>`, entirely independent of `db_agents`. Identity-confirmer in `blockcontroller/persistent.rs` layers a *second* slug comparison on top for jekt sender verification. |
| 2 | **Work Queue** | `backend/storage/work_queue.rs`, `server/work_queue.rs`, `agentmux-mcp/src/main.rs` (WorkEnqueue/Claim/Heartbeat/Complete/Release) | Raw SQL string comparison: `target_agent`, `claimed_by` columns hold the slug directly. No resolution anywhere in this path — the starkest example. |
| 3 | **S1 WS-RPC authorization** | `server/app_api/mod.rs` (`check_s1`), ~24 call sites across `bundle.rs`, `identity.rs`, `mcp.rs`, `memory.rs`, `skill.rs` | `ctx.agent_id != req_agent_id` — a raw string-equality **authorization** decision, not just a data lookup. Highest-severity category, since a collision here is an access-control bug, not just a wrong-answer bug. |
| 4 | **Cron** | `backend/cron/mod.rs`, `agentmux-mcp/src/main.rs` (CronCreate) | `target` fires through the reactive-handler registry (#1) at trigger time — same root map, different entry point. |
| 5 | **Frontend block registration** | `frontend/app/view/term/termagent.ts`, `frontend/app/view/agent/agent-view.tsx` | POSTs the slug to `/reactive/register`, i.e. this is the frontend half of #1, not a separate registry. |
| 6 | **muxbus / WAN / cloud relay** | `muxbus/relay.rs`, `muxbus/cloud_subscriber.rs`, `muxbus/agent_credentials.rs` | **Deliberately excluded from this migration — see §5.** The slug is the only identity concept that currently has any meaning *across machines*; `db_agents.id` is a per-machine-local UUID with no cross-machine relationship to "the same" logical agent on another host. |
| 7 | **bashwrap cwd-state** | `agentmux-bashwrap/src/bash_wrap.rs` (`cwd_state_path`) | Already downgraded to a fallback (`AGENTMUX_INSTANCE_SLUG` preferred) after a prior related bug. Low priority; folds into Phase 5 cleanup, not its own phase. |

### Already-existing resolvers (the pattern to consolidate, not rebuild)

Three independent, slightly-inconsistent slug→id resolvers already exist,
all in the one subsystem family that already got this right:

- `Store::instance_get_by_slug` (`backend/storage/agents.rs:1953-1975`) —
  `SELECT ... WHERE slug = ?1 ... ORDER BY updated_at DESC LIMIT 1`. Silently
  recency-tiebreaks duplicates instead of erroring.
- `native_memory_handlers::resolve_agent_uuid` / `memory_dir_for_agent`
  (`server/native_memory_handlers.rs:104-202`) — backs Memory* MCP tools.
- `app_api::resolve_agent_definition_id` (`server/app_api/mod.rs:2751-2799`)
  — backs Identity*/PresetGet.

All three do the same conceptual thing (slug → `db_agents.id`) with
slightly different fallback behavior and no shared implementation. Phase 0
(§6) consolidates them into one function every other phase calls.

---

## 2.5 Verification pass against `main` @ `059cc6e` (2026-09-22)

The inventory in §2 was written against `4ab7a9ea9`. PR #3480 landed after
that, so this section re-verifies §2 against current `main` and corrects it.
**Three claims changed; one defect was missed entirely.** The phase plan in
§6 survives intact — nothing below reorders or removes a phase.

### 2.5.1 Phase 0 — #3480 did *not* partially deliver it

> **Status update.** As of the PR that adds `backend/agent_resolve.rs`, Phase 0's
> shared resolver **now exists** and `resolve_agent_uuid` /
> `resolve_agent_definition_id` both delegate to it. The remaining Phase 0 item
> was the §4 safety net (`instance_create`, per §2.5.2), which landed in #3514
> — the data check §4 asked for found no existing collisions to clean up, so it
> closed a contract gap rather than repairing data. **Phase 0 is now complete.**
> `backend::history`'s inline
> fourth resolution path is Phase 1's, not Phase 0's — its extra raw-slug
> fallback is load-bearing and needs its own change. The rest of this section
> records the state before that, and why #3480 was not it.

Easy to misread, so stated explicitly: #3480 created
`backend/agent_registry_lookup.rs`, which consolidates three copies of a
**registry-record** lookup (`find_active_record_by_slug`) into one
fail-closed implementation. That is a *parallel* resolver over the
in-memory named-agent registry — **not** one of the three slug→`db_agents.id`
resolvers §2 lists, and not Phase 0.

All three resolvers named in §2 are still separate on `main`, and all three
still bottom out on the unguarded `Store::instance_get_by_slug`:

- `Store::instance_get_by_slug` — `backend/storage/agents.rs:1959`
- `native_memory_handlers::resolve_agent_uuid` — `server/native_memory_handlers.rs:189`
- `app_api::resolve_agent_definition_id` — `server/app_api/mod.rs:2772`

What #3480 *does* give Phase 0 is a proven precedent to copy: its
fail-closed-on-ambiguity shape (`agent_registry_lookup.rs:64-77`) is exactly
the `ResolveAgentError::Ambiguous` behavior §4 asks for, already reviewed and
merged. Phase 0 should mirror it rather than invent a second convention.

### 2.5.2 The §4 safety net half-exists — but `agent_def_insert` is what protects today

§4 asks to "enforce slug uniqueness going forward at agent-creation time
(reject/rename-suffix a colliding slug at creation)". Half of that is
already built and must not be rebuilt: `agent_def_insert`
(`backend/storage/agents.rs:592`) scans `db_agents` for `base` / `base-N`
and suffix-resolves before inserting.

The hole is a different function. **`Store::instance_create`
(`backend/storage/agents.rs:1568`) performs no slug collision handling at
all**, and per its own doc comment (`:1555-1559`) *"Only a fresh launch of a
TEMPLATE creates a new row"* — that new row copies the template's slug
verbatim. `migrations.rs:287-290` records this as deliberate: no unique index
on `db_agents(slug)` is added precisely *because* template-launch projections
are expected to share their template's slug.

> **Corrected after running §4's data check.** This section originally claimed
> *"launching the same template twice produces two `db_agents` rows … both
> carrying the template's slug"*, and #3500 shipped citing that as its
> reproduction. **The launch flow does not do this.** Corrected below; the
> two paragraphs above remain accurate.

**The data check §4 asks for, run across every AgentMux database on one host:**

```
databases scanned                          : 47
db_agents rows                             : 202  (184 templates, 18 launches)
databases with a duplicate resolvable slug : 0
slugs colliding case-insensitively         : 0
collision-suffixed slugs (-2, -3, …)       : 0
max launches of any ONE template           : 3
launch slug == its template's slug         : 0  (differs: 18)
```

Zero collisions — **including a template launched three times**, the exact
case the original claim said would produce duplicates.

The reason is that a real launch supplies a *name*:

```
launch  slug='parlo'   name='Parlo'    is_seeded=0  parent_template_id → claude
launch  slug='maricon' name='Maricon'  is_seeded=0  parent_template_id → claude
  template  slug='claude'  name='Claude'  is_seeded=1
```

Every launch slug derives from the launch's own name. Supplying a name creates
a user definition through `agent_def_insert` — which *does* collision-resolve —
and `instance_create` then folds into that row (`key = def.id`, since
`def.is_seeded == 0`). The verbatim-copy branch (`key == inst.id`, binding
`def.slug`) only runs when `definition_id` names a **seeded template
directly**, which the launch flow never does.

So this section had it backwards: `agent_def_insert`'s collision scan is not
the already-solved half to be left alone — **it is the thing actually
preventing duplicate slugs today**. `instance_create`'s missing guard is a gap
in that function's contract, reachable by calling it directly (as #3500's
regression test does, deliberately), but not a path the application takes.

**Consequences:**

- The §4 safety net is **lower priority** than this section first implied. It
  closes a contract gap, not an occurring fault.
- §4's deferred decision — audit-and-rename existing collisions, or rely on
  `Ambiguous` — is **moot on this evidence**: there are no existing collisions
  to rename.
- #3500 remains correct: no `UNIQUE` constraint exists, `instance_create` has
  no guard, and the old `ORDER BY updated_at DESC LIMIT 1` did silently pick a
  winner. What changes is its urgency — it hardens a reachable-but-unobserved
  state rather than closing an actively-firing disclosure.
- #3480's registry-side collisions are a different mechanism (two hosts,
  `instance_name` casing) and are **unaffected** by this correction.
- One host is a sample, not proof. Re-run elsewhere before generalising:
  `SELECT slug, COUNT(*) FROM db_agents WHERE is_template=0 AND user_hidden=0
  AND slug<>'' GROUP BY slug HAVING COUNT(*)>1`.

Method note, since this is the second correction of its kind in this document
(see also §2.5.5): both errors came from reading one function in isolation and
inferring the system's behaviour from it, rather than tracing its callers or
checking the data. Prefer the data check first where one is cheap.

### 2.5.3 The registry guard was unverified — now covered

As first written this section recorded that `backend/agent_registry_lookup.rs`
shipped **zero tests**, leaving #3480's fail-closed guard — the one §2.5.1
recommends Phase 0 mirror, and which several callers now depend on — entirely
unverified.

Closed by the suite added alongside this spec revision: single-match
resolution, `derive_slug`-on-both-sides matching (the #2428 bug class),
not-found, the two-records-one-slug refusal, and
`find_active_record_by_slug_and_definition` still resolving the collision the
slug-only lookup declines. The refusal test was confirmed to fail when the
guard is reverted to returning an arbitrary match, so it cannot pass
vacuously.

Phase 0's own resolver still needs its own equivalent coverage — this only
settles the registry half.

### 2.5.4 Corrected counts and status

- **S1 call sites: 22, not ~24** — `memory.rs` 3, `mcp.rs` 7, `skill.rs` 6,
  `bundle.rs` 2, `identity.rs` 4. Only the 2 in `bundle.rs` resolve (via
  `resolve_agent_for_s1`); the other 20 pass the slug straight to `check_s1`.
- **Subsystem 7 (bashwrap) is done**, as §2 anticipated — `bash_wrap.rs:175-207`
  keys on `AGENTMUX_INSTANCE_SLUG` with `AGENTMUX_AGENT_ID` as a documented
  legacy fallback. Phase 5 inherits only the optional fallback removal.
- **Subsystems 1–5 are unchanged** from §2's description.

### 2.5.5 Missed defect: the Work Queue has no *per-agent* authorization

> **Corrected.** This section first claimed the work-queue endpoints had "no
> authenticated context" and were reachable from the LAN. That was wrong —
> they sit inside `auth_middleware` and require a valid `X-AuthKey`. The
> error was reading the handlers and the storage layer without reading the
> router's layer stack. Corrected below and in issue #3501; the defect is
> real but narrower than first written.

§2 #2 and §6 Phase 3 treat the work queue as a *correctness* problem — the
columns hold slugs and compare by raw SQL equality. That is true but
understates it.

`handle_work_claim` (`server/work_queue.rs:154`) reads `agent_id` from the
**request body** and validates only that it is non-empty. Nothing compares it
against the caller's own identity. The same pattern holds for the
enqueue/heartbeat/complete/release handlers (`:138`, `:174`, `:237`, `:251`,
`:269`), and the ownership checks in
`backend/storage/work_queue.rs:262,287,324` are raw equality against whatever
string the body supplied.

**What authentication does and does not give you.** The routes are registered
inside the `auth_middleware` layer (`server/mod.rs:682-689`, layer at `:691`),
so a valid `X-AuthKey` is required. But that middleware compares against
`state.auth_key` — a single **server-wide** value (`AppState.auth_key`, seeded
from config at `bootstrap.rs:1778`). It authenticates *"some caller on this
instance"*, never *"which agent"*, and every agent already holds that key
because it is how `agentmux-mcp` reaches the server at all
(`AGENTMUX_AUTH_KEY`).

So agent A, using entirely legitimate credentials, can claim, complete, cancel
or heartbeat work targeted at agent B by naming B's slug. This is a
privilege-separation gap *between agents*, not a perimeter one — the same
distinction `check_s1` exists to enforce on the WS-RPC surface, where holding
the channel key is deliberately not sufficient to act as another agent.

§7 asks Phase 3 to prove "`WorkClaim` ownership can't be spoofed by a
same-named agent". Today it can be spoofed by *any agent on the instance* that
names the target's slug — no collision required. **Canonicalizing these
columns to `db_agents.id` raises the bar (a UUID is unguessable where a slug
is not) but does not close the hole**, and treating Phase 3 as the fix for it
would leave an authorization gap behind a migration that looks like it
addressed it. This wants its own fix, independent of and ideally before
Phase 3.

The honest obstacle: this API surface has **no per-agent identity at all** to
check against today. Unlike the WS-RPC path, which authenticates an agent into
an `RpcContext`, the work queue has only the shared key — so a per-agent
credential or equivalent context has to exist before any ownership check can
be written. That is the design decision this fix turns on, and it is why the
fix is not a small one. Tracked as #3501.

Related, same subsystem: issue #3276 (work-queue rows targeted at a deleted
agent become permanently unclaimable).

---

## 3. Design principle: resolve once at the trust boundary, don't push UUIDs into human/model-facing surfaces

**This migration does not mean `SendMessage(to: "<uuid>")`, `WorkClaim`
requiring a UUID, or any MCP tool argument changing shape.** Typing or
reading a UUID in a chat transcript is worse for both the human operator and
the calling model than a name, and nothing about fixing the identity bug
requires that regression.

The fix is the pattern the Memory/Identity subsystem (§2, "already-existing
resolvers") already uses correctly: **the human-readable slug stays the
argument shape everywhere it is today; the server resolves it to
`db_agents.id` exactly once, at the first point it crosses into a
lookup/comparison/storage operation, and every operation after that point
uses the id, never the slug again.** A slug is a *display* value and an
*input* value; it should never be a *storage or comparison* value. Category
(a) sites are exactly the sites currently violating that.

This also resolves the ambiguity concern from `instance_get_by_slug`'s
current behavior: once resolution happens once, at one place, per phase,
"which agent did this slug mean" is answered exactly once and consistently
— not re-derived (potentially differently) at every downstream site the way
it partially is today.

---

## 4. Phase 0 — the shared resolver (prerequisite for every other phase)

Before any category-(a) site can be migrated, one canonical function must
exist:

```rust
// agentmux-srv/src/backend/storage/agents.rs, or a new identity module
pub fn resolve_agent_id(&self, slug_or_id: &str) -> Result<String, ResolveAgentError>;
// ResolveAgentError::NotFound | ResolveAgentError::Ambiguous(Vec<String>)
```

- Accepts either a slug or an already-resolved id (idempotent — many
  call sites will pass through values that started as one or the other
  depending on migration phase ordering, and this must not require every
  caller to know which).
- Consolidates and replaces the three existing resolvers in §2 — each of
  their call sites switches to this one, behavior audited for any
  intentional difference (e.g. `instance_get_by_slug`'s `is_template = 0 AND
  user_hidden = 0` filter — confirm whether that filter belongs in the
  shared resolver unconditionally or needs to stay a caller-supplied
  option).
- **Changes the ambiguity behavior**: an unresolvable duplicate slug returns
  `Ambiguous`, not a silent recency pick. This is a deliberate behavior
  change from `instance_get_by_slug`'s current fallback and needs a decision
  on what callers do with it (reject the request with a clear error asking
  the human to disambiguate, most likely — needs UX input, not resolved
  here). **Mirror `agent_registry_lookup.rs:64-77`'s already-merged
  fail-closed shape rather than inventing a second convention — see §2.5.1.**
- **Add a migration-window safety net**: enforce slug uniqueness going
  forward at agent-creation time (reject/rename-suffix a colliding slug at
  creation), even though the long-term fix makes slug collisions merely
  cosmetic rather than dangerous. This matters because phases land
  sequentially — sites in a not-yet-migrated phase still compare by slug
  until their own PR lands, so new collisions should stop being introduced
  during the transition window. **Per §2.5.2 this work targets
  `instance_create`, not `agent_def_insert` — the latter already
  suffix-resolves; the former is where template launches mint duplicates.**
  Existing collisions (if any exist on real installs) need a decision:
  audit-and-rename, or leave and rely on `Ambiguous` handling — needs a data
  check before deciding, not assumed here.

**This phase has no user-visible behavior change** (existing single-slug
call sites keep working identically) and should ship first, alone, as its
own PR — every later phase's diff becomes "call the shared resolver here"
rather than "invent resolution logic here," which keeps each subsequent PR
small and low-risk.

---

## 5. Explicit non-goal: WAN / cross-machine addressing is not migrated

`muxbus/relay.rs`, `cloud_subscriber.rs`, and `agent_credentials.rs`
address agents by slug in `X-Agent-ID` headers and REST path segments, and
this is **not a shortcut to fix** — it's the only identity concept that
currently has cross-machine meaning at all. `db_agents.id` is generated
independently per machine's local SQLite `objects.db`; there is no existing
relationship between "agent X's id on machine A" and "the same logical agent
X's id on machine B." Migrating WAN addressing to `db_agents.id` as written
would not fix anything — it would just replace one arbitrary string with
another arbitrary, *differently-arbitrary-per-machine* string, while
actually breaking cross-machine routing (today's slug at least matches by
convention when a human names two instances the same thing on purpose).

A real fix for WAN-tier identity needs a genuinely new concept — a
cloud-registered, account-scoped stable agent id (the same shape
`SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md` already identifies
`account_user_id` as the natural cross-machine sync-scope key for a
different subsystem) — and is out of scope here. This spec's phases (§6)
are all same-machine, local-only subsystems. If WAN identity is wanted
later, it should be its own spec building on this one's local resolver as a
building block, not bolted on here.

---

## 6. Phased PR plan

Each phase is sized to land as one PR. Later phases depend on Phase 0;
otherwise phases are independent of each other and can land in any order —
suggested order below is roughly risk-ascending (lowest-blast-radius
first), not a hard dependency chain except where noted.

### Phase 0 — shared resolver (§4)
No user-visible change. Prerequisite for all phases below.

### Phase 1 — Memory/Identity/Preset consolidation
Lowest risk: these subsystems are *already* id-correct, this phase only
de-duplicates their three separate resolvers down to Phase 0's one function.
Good first real PR after Phase 0 — proves the shared resolver works
end-to-end against call sites that already pass all their existing tests,
before touching any subsystem that's actually broken today.

### Phase 2 — Reactive handler registry (§2 #1)
The big one. `agent_to_block`/`agent_info` (`reactive/handler.rs`) become
keyed by `db_agents.id`. `register_agent_with_nonce` resolves the incoming
slug via Phase 0 before inserting. `handle_reactive_message` resolves
`target_agent`/`source_agent` via Phase 0 before any lookup, comparison, or
audit-log write that currently uses the raw string for matching (audit-log
*display* can keep showing the human-readable slug — resolve for lookup,
render for display, per §3). The `blockcontroller/persistent.rs` identity-
confirmer's stable-identity comparison migrates alongside this, since it's
the same registry's data. Frontend Phase 5 (`/reactive/register` callers)
should land together with this phase or immediately after, since they're
the same wire contract.

**Migration-window risk — resolved, 2026-09-22: registry rebuild-on-restart
is sufficient, and there is nothing in flight to lose.** This section asked
whether in-flight jekts queued before deploy could reference the old
slug-keyed shape. Traced through the code:

- **No local queue exists.** `inject_message` is synchronous — it returns
  `InjectionResponse { success: false, error }` when the target does not
  resolve. There is no deferred store, retry buffer or pending table; a jekt
  either lands or fails to its caller immediately.
- **The registry is in-memory only** (`reactive/handler.rs`'s `HashMap`s), so
  a restart empties it and agents re-register on reconnect — which already
  happens on every srv restart today.
- **The one durable queue is server-side in the cloud** and is
  claim-before-deliver: `cloud_subscriber.rs` `POST /reactive/ack`s an
  injection *before* delivering it, and releases it back to pending if local
  delivery fails. Its own comment records the prior bug where delivering first
  let two pollers double-deliver. So a deploy mid-delivery releases rather
  than drops.
- That cloud queue is keyed by slug and **stays** that way: it is muxbus/WAN,
  which §5 permanently excludes. It is therefore not a migration window at all
  but a standing interface, handled by resolving `target_agent` on the way in
  like every other entry point.

**Implemented differently from the sketch above.** Rather than rekeying the
maps at each call site, the handler takes an injected `AgentKeyResolver`
(matching its existing `InputSender`/`AgentIdentityConfirmer` seams, so
`backend::reactive` gains no storage dependency) and resolves in one place,
`Handler::canonical_key`. There are ~10 entry points into this handler —
Slack, Discord, Telegram, WhatsApp, the cloud subscriber, fleet, pane, the
HTTP routes and the MCP — and resolving at each would have reproduced exactly
the per-site drift §2 blames for this defect being found four times.

Unresolvable agents fall back to the lowercased slug, i.e. the pre-Phase-2
key, so an agent with no `db_agents` row keeps working rather than dropping
off the registry. With no resolver installed the behaviour is byte-identical
to before, which is what keeps the pre-existing tests meaningful.

The alias registry is deliberately **not** canonicalized: aliases are the
stable `AGENTMUX_AGENT_ID` by design, so `inject_message_inner` now carries
two keys — canonical for the primary map, raw slug for the alias map.

**The slug → key binding is pinned at registration, and lookups never re-run
the resolver** (ReAgent P1 on #3520). Resolving on every lookup would read the
store each time, so a transient failure there — a busy or locked SQLite read —
would fall back to the lowercased slug, which no longer matches the
canonical-id key the entry is stored under. The lookup would miss and report
"agent not found" for an agent that is registered and healthy: this phase's own
failure mode, re-introduced through resolver flakiness rather than slug
collision. Pinning once makes every later lookup independent of store
availability.

A consequence worth stating: when two agents register under one slug, that slug
becomes `Ambiguous` and addresses **neither**, rather than whichever registered
last. Each remains reachable by its canonical id. That is the same fail-closed
rule `instance_get_by_slug` and `find_active_record_by_slug` already follow, now
applied to message routing.

### Phase 3 — Work Queue (§2 #2)
`target_agent`/`claimed_by` columns store `db_agents.id`. Resolution happens
at `WorkEnqueue`/`WorkClaim` time (server-side, via Phase 0), so the MCP
tool arguments stay human-readable slugs.

**Migration-window risk:** existing rows in `db_work_queue` (if any are
live/unclaimed at deploy time) hold slug strings in these columns under the
old schema. Given work-queue items are expected to be short-lived
(claim/heartbeat/complete within a bounded window per their own design),
the simplest safe approach is likely a one-time drain-or-expire of any
items still unclaimed at migration time rather than a value-rewriting
migration — needs confirming against actual queue semantics/TTLs before
deciding, not assumed here.

**This phase is not the fix for §2.5.5's authorization gap** — that is a
separate defect in the same files and should land on its own, ideally first.

> **Recommendation, 2026-09-22: do not proceed as written yet.** §2.5.5's root
> cause is now understood — the work queue has no per-agent identity to check
> against because `AGENTMUX_AUTH_KEY` is instance-wide and inherited, which
> `backend/pane_env.rs` and
> `docs/retro/retro-env-inheritance-instance-isolation-breach-2026-09-17.md`
> (recommendation 4) already track as a redesign, not a bug fix. Tracked as
> #3501.
>
> That changes this phase's cost/benefit. Canonicalizing these columns makes
> the asserted identity unguessable but still unverified, so it buys hardening
> rather than a fix — while carrying a migration for live unclaimed rows that
> the per-invocation-handoff redesign may partly invalidate. The consistency
> win is real; it is simply not worth doing *before* the identity decision,
> and it risks reading as having closed #3501 when it has not.

### Phase 4 — S1 WS-RPC authorization (§2 #3)

> **Implemented, but not the way this section proposed.** Resolution happens
> inside `check_s1` — both sides, at the single point of comparison — rather
> than by stamping `RpcContext.agent_id` at `bus:register`. Rationale below;
> the original proposal is kept for the record because the hazard it flagged
> is exactly what drove the change.

As originally written: highest-severity category (this is an authz boundary,
not just a lookup), but mechanically simple once Phase 0 exists —
`RpcContext.agent_id` gets stamped with the *resolved* id at `bus:register`
time (`server/websocket.rs`) instead of the raw slug, and `check_s1`'s
equality check is then comparing ids on both sides unchanged. The 22 call
sites (§2.5.4) need no individual changes if `req.agent_id` is *also*
resolved before reaching `check_s1` — *"if only one side of the equality is
resolved and the other isn't, this phase would silently and permanently break
authorization for every caller rather than just failing loudly."*

**Why the implementation diverged.** That warning is not a risk to be verified
away; it is inherent to splitting the two sides across two places. Stamping at
registration migrates the caller's side at deploy time, while the request
side changes only when every MCP client and frontend caller starts sending
ids — so there is a window, of unbounded length, in which the two sides carry
different forms and every S1 call is denied. No amount of care at
implementation time removes it, because the two halves are not deployed
together.

Resolving both sides inside `check_s1` has no such window: whichever form
either side happens to carry, the comparison is made on canonical ids.
Specifically:

- A byte-equality fast path runs first, so the common case costs no store
  lookup and behaves *identically* to the pre-migration comparison.
- Resolution only runs where the old code would already have returned
  `FORBIDDEN`. This phase can therefore admit calls that used to be rejected,
  and can never reject one that used to be admitted.
- Admitting more is safe because resolution is per-input and fails closed: two
  inputs resolve to one id only when they name one agent, and an ambiguous
  slug resolves to `Err` on *both* sides, so a collision denies rather than
  granting one of the two agents' access.
- A failed resolution reports `agent_id mismatch`, never "no such agent", so
  the boundary cannot be used to probe which agents exist.

`check_s1` takes the store rather than `&AppState` because the handlers do not
agree on what they capture — some clone the whole state into the closure,
others clone `mstore` alone — and a borrowed `&AppState` cannot escape into a
`'static` future.

Per §7, the tests pin the deny direction, not just the allow path. Two
mutations were run to confirm they are load-bearing: skipping resolution
entirely (admit anything) fails exactly the three deny tests, and the
half-migrated comparison this section warned about — resolved caller against
raw request value — fails exactly the one test written for it and nothing
else.

### Phase 5 — Cron + frontend registration + bashwrap cleanup
Smaller follow-ups, safe to batch into one PR: `CronCreate`'s `target`
resolves via Phase 0 before storing (rides on Phase 2's registry once
that's merged, so this phase should land after Phase 2); frontend
`termagent.ts`/`agent-view.tsx` `/reactive/register` payload shape
confirmed against Phase 2's final wire contract; `bash_wrap.rs`'s
`AGENTMUX_AGENT_ID` fallback path removed now that the primary
`AGENTMUX_INSTANCE_SLUG` path is the only one needed (or left as
belt-and-suspenders — low-stakes either way, a judgment call for that PR).

> **Correction, 2026-09-22: the bashwrap cleanup is not low-stakes, and the
> "only one needed" premise is wrong.** `AGENTMUX_INSTANCE_SLUG` is set in
> exactly one place — `frontend/app/view/agent/agent-model.ts` — while
> `AGENTMUX_AGENT_ID` is set in several, including the MCP server env written
> by `backend/agent_config.rs`. So the fallback is load-bearing for every
> launch path the frontend does not drive; removing it silently re-keys their
> cwd-state onto the cwd-derived third fallback.
>
> That may well be fine — `cwd_state_path`'s own comment says the cwd is
> "always the instance's fixed home directory either way" — but that is a
> claim to verify per launch path, not to infer from a comment, and the
> verification is the actual work rather than the deletion. Until it is done,
> leaving the fallback in place is the safe option.
>
> The cron and frontend-registration halves of this phase remain blocked on
> Phase 2 regardless.

---

## 7. Testing / verification plan (applies per-phase, not just once at the end)

- **Phase 0:** unit tests for `resolve_agent_id` covering: unique slug hit,
  id-passthrough hit, not-found, and the new `Ambiguous` case (two
  definitions sharing a slug) — the ambiguity already existed in
  `instance_get_by_slug` long before any test covered it. **The registry
  half of that gap is now closed (§2.5.3); Phase 0's own resolver still
  needs the equivalent.** Build the duplicate-slug fixture as two
  non-template rows sharing a slug, via `instance_create`. Note per §2.5.2
  that this exercises a gap in that function's *contract* and is **not** a
  state the launch flow produces — the test is legitimate, but do not cite it
  as evidence the fault occurs in practice.
- **Every subsequent phase:** a regression test asserting the specific bug
  class this spec exists to prevent — two agent definitions sharing a
  slug, verify the phase's subsystem no longer cross-talks between them
  (e.g. Phase 2: two agents named identically, confirm `SendMessage`
  reaches the intended recipient, not "whichever registered most
  recently"; Phase 3: confirm `WorkClaim` ownership can't be spoofed by a
  same-named agent — **and, per §2.5.5, that it cannot be spoofed by an
  unrelated caller either, which is a different assertion**; Phase 4:
  confirm S1 auth actually still denies a mismatched caller after the
  resolve-before-compare change, not just that it still allows a matched
  one).
- **End-to-end, after Phase 2 specifically:** rerun
  `SPEC_CROSS_CHANNEL_AGENT_HISTORY_RESOLUTION_2026_09_21.md`'s own Bug-A
  regression test against the now-shared resolver, confirming that spec's
  fix and this one converge on the same mechanism rather than solving the
  same problem twice in two different ways.
- **Non-goal guard:** an integration test confirming WAN/muxbus jekt
  routing (§5) is unaffected by any phase here — cross-machine
  send/receive between two simulated instances should behave identically
  before and after this entire migration.

---

## 8. Confidence

- **High — directly verified via full-codebase audit, not sampled:** the
  category (a) inventory in §2. Every subsystem listed was read in its
  actual current implementation, not inferred. Re-verified against
  `059cc6e` in §2.5; subsystems 1–5 unchanged.
- **High:** the WAN/muxbus non-goal reasoning in §5 — confirmed directly
  that `db_agents.id` is generated per-machine with no cross-machine
  relationship, and that slug is the only field currently serving that
  role.
- **High, and corrected downward:** §2.5.2. That `instance_create` has no
  slug guard is read directly from its implementation and stands. The
  *reproduction* built on it — that a real template launch therefore shares
  its template's slug — was inferred from that function plus
  `migrations.rs:287-290`, without tracing the launch flow into it, and the
  data check disproved it (0 collisions in 202 rows; 18 launches, none
  inheriting a template slug). Confidence in a claim read off one function is
  confidence about that function, not about the system.
- **Medium, flagged as open questions rather than guessed at:** the exact
  transition behavior for in-flight state during Phase 2 (registry
  rebuild-on-reconnect) and Phase 3 (work-queue row handling at deploy
  time), and whether every side of Phase 4's `check_s1` equality check
  needs simultaneous resolution or can be staged — these need a decision
  made deliberately during each phase's own implementation, not assumed
  here.
