# SPEC: agent identity is carried, never derived

**Date:** 2026-09-23
**Status:** proposed — redesign of
`SPEC_CANONICAL_AGENT_ID_MIGRATION_2026_09_21.md` after its Phase 2 was
implemented and proven unable to fix the defect it targeted. Supersedes that
spec's §6 phase plan; its §2 inventory and §5 WAN analysis remain valid and are
cited rather than restated.
**Trigger:** Repo owner, after Phase 2's failure: *"redesign the entire
solution... the goal is agent id unique... do the most robust solution."*
**Evidence base:** five review rounds on #3520 (four P1s, one P0), a data check
across 47 live databases, and direct measurement of `resolve_agent_id`'s real
behaviour.
**Revision 2 (2026-09-23).** Revision 1 was reviewed and **four of its
load-bearing empirical claims were false**. §0 records them; §4, §5, §6, §7 and
§9 are rewritten. The principle in §2 survived review unchanged; the mechanism
did not. Read §0 first — its corrections are why the rest looks as it does.

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
2. **Identity is proven, not asserted.** At any trust boundary, identity comes
   from the connection, never from the request body (§6).
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

### 4.1 The agent asserts its own identity; the server verifies it

Revision 1 had the frontend send the UID. §0.1 rules that out: the frontend's
`block.meta.agentInstanceId` is documented as going stale on pane reuse, which
is the respawn case this design must survive.

**The process that knows its identity is the agent itself**, because the server
told it at spawn. So:

- Spawn injects `AGENTMUX_AGENT_UID` (the agent's `db_agents.id`) and
  `AGENTMUX_AGENT_TOKEN` (§6), both minted by the server that created the row.
- The agent registers **itself**, presenting the token.
- The server derives the UID **from the token**, not from the payload. A UID in
  the body is a hint at most; the token is the identity.

This also answers a defect revision 1 did not know it had: the frontend starts
the controller *before* `CreateAgentInstanceCommand` completes, inside a
best-effort `try/catch`, and `instance_create` may legitimately return a
different id than the caller passed (`storage/agents.rs:1541-1546`). Any
frontend-asserted UID therefore races and can be absent or wrong. An
agent-asserted, server-verified one cannot: it does not exist until the row
does.

The frontend's registration calls become presence/lifecycle signals keyed by
`block_id` — which it genuinely owns — and stop carrying identity at all.

### 4.2 What moves, and what must move together

| Boundary | Today | Becomes |
|---|---|---|
| Spawn env | `AGENTMUX_AGENT_ID` (two meanings, §0.2) | `AGENTMUX_AGENT_UID` + `AGENTMUX_AGENT_TOKEN`; name/slug split explicitly |
| Registration | frontend POSTs a display name | agent POSTs with its token; server resolves token → UID |
| Registry keys | lowercased display name | UID |
| **Identity confirmers** | compare display names (`handler.rs:534-560`) | compare UIDs — **same change, or delivery breaks** |
| **Alias map** | stable display name | UID; the alias concept disappears (§4.3) |
| Work queue, cron | slug | UID, captured at authoring time (§5.4) |
| muxbus / WAN | slug | UID (§8) |

The two bolded rows are why revision 1's §4 would have failed in production:
re-keying `agent_to_block` while `agent_identity_confirmer` still compares
display names makes every delivery an `"identity mismatch"`. They are one
atomic change, not three phases.

### 4.3 The alias map disappears, and that is the point

`alias_to_block` exists because the primary key is a *renameable* display name,
so a second, stable key was bolted alongside it
(`INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`). Once the primary key is a UID,
"stable alternative key" is what the primary key already is. The alias map, its
inverse, and their eviction rules are deleted rather than migrated.

That is the clearest evidence the design is addressing the root cause: a whole
subsystem that existed only to compensate for the defect stops being needed.

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

**Expiry must not strand a running agent.** A token that expires mid-task turns
a healthy agent silent, which is the failure mode #3520's first P1 already
demonstrated is easy to create and hard to notice. Tokens are therefore
long-lived for the process lifetime and invalidated on agent deletion, rather
than short-lived and renewed — renewal is a liveness dependency on the very
path being secured. Revoking on deletion is sufficient because the UID is never
reused.

### 6.4 Bootstrapping

The token is injected at spawn by the server that created the row. There is no
chicken-and-egg: the agent never authenticates to *obtain* it. Its
confidentiality rests on process environment isolation, which `pane_env.rs`'s
`PANE_ENV_KEEP` already governs and which this design tightens by replacing an
instance-wide credential with a per-agent one — an inherited copy stops being
omnipotent, exactly the retro's stated fallback position.

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
- **M1 — Mint and carry.** Server mints the UID and token at row creation and
  injects both at spawn; add UID columns alongside slug columns; dual-write.
  **No reader changes.** Revertible by ignoring the new fields.
- **M2 — Registration and delivery, atomically.** The agent registers itself
  with its token; the server derives the UID from the token; the registry,
  **both identity confirmers**, and the alias map all move in the *same*
  change (§4.2). Splitting them rejects every delivery as an identity
  mismatch. Slug fallback retained **with a counter** (§9.2).
- **M3 — Work queue and cron.** Columns hold UIDs; one-time backfill of live
  rows. Resolution moves to the MCP boundary so tool arguments stay readable.
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

## 10. Salvage from #3520

Four fixes are bugs in `main` today, independent of how keying is redesigned.
They should land on their own rather than waiting for this spec:

1. **`record_supervisor_decision` never resolved its target.** Nudge ceilings,
   audit `block_id`s and respawn-staleness checks are all silently wrong right
   now — two agents sharing a name share a nudge ceiling.
2. **Teardown must not inherit delivery's fail-closed rule** — otherwise
   `unregister` silently no-ops while reporting success.
3. **Bindings derived from live state, not from a previously-stored key** —
   otherwise one transient failure permanently poisons an agent.
4. **No store read on the lookup path** — subsumed by this design, but the test
   that pins it is worth keeping.

## 11. What this spec is not

- **Not a rename of everything to UUIDs.** §5.3: humans and models keep names.
- **Not a new identity.** §1.2: the UUID already exists on every real agent.
- **Not incremental repair of the predecessor.** Its Phase 2 cannot be patched
  (§1.1); the phases here replace that plan rather than continuing it.

## 12. Confidence

**Revision 2 note.** Revision 1 was reviewed adversarially and four of its
load-bearing empirical claims were false (§0). Every claim below that begins
"measured" was re-run against `main` for this revision.

Revision 2 then repeated the mistake once, which is worth recording precisely
because it happened *inside* the correction: §6.3's original permissions claim
was taken from that review and written in without being checked, and it was
false. Reviewers are not a substitute for running the code — including
reviewers who are right about everything else. Verify what you propagate, not
just what you author. The pattern in both the
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
