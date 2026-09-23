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
behaviour. Every empirical claim below was run, not reasoned.

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

**The slug stops being an identity.** It becomes a display attribute with no
uniqueness requirement at all: `agent_def_insert`'s `-2`/`-3` suffixing is
deleted (§7.5), because suffixing only ever existed to fake uniqueness for a
field that should not have carried it. Two agents may be called the same thing,
exactly as two people may.

`derive_slug` survives for one purpose only: matching in the disambiguation UI
(§5.2).

## 4. Carrying it

| Boundary | Today | Becomes |
|---|---|---|
| Spawn env | `AGENTMUX_AGENT_ID` = slug | adds `AGENTMUX_AGENT_UID` = UID |
| MCP → srv | sends slug | sends UID |
| Frontend → `/reactive/register` | sends display name | sends UID |
| `input.rs` re-register | `block.meta["agentName"]` | `block.meta["agentInstanceId"]` |
| Reactive registry keys | lowercased display name | UID |
| Work queue `target_agent`/`claimed_by` | slug | UID |
| Cron `target` | slug | UID |
| muxbus / WAN | slug | UID (§8) |

Two facts make this tractable, both verified:

- **The frontend already holds the UID.** `agent-model.ts:823` sets
  `meta: { agentInstanceId: inst.id }` on the block. It has been carrying the
  right value all along and sending the wrong one.
- **Per-agent credential machinery already exists.**
  `muxbus/agent_credentials.rs::ensure_agent_credential` provisions and caches
  per-agent tokens. §6 builds on it rather than inventing a parallel scheme.

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

## 6. Identity is proven, not asserted

Closes #3501, and the env-inheritance retro's recommendation 4, together.

Today `AGENTMUX_AUTH_KEY` is instance-wide and inherited by every pane
(`pane_env.rs` says so in its own keep-set comment). It authenticates *"some
caller on this instance"* and can never answer *"which agent"*. So every
authorization decision that reads `agent_id` from a request body is
unenforceable by construction — the work queue's most visibly, but the property
is general.

**Design:**

- At spawn, mint a per-agent token bound to the UID, short-lived and renewable,
  reusing `agent_credentials.rs`.
- Injected as `AGENTMUX_AGENT_TOKEN`, replacing `AGENTMUX_AUTH_KEY` in
  `PANE_ENV_KEEP` for agent panes.
- Server maps token → UID on every request. **That** is the caller's identity.
- Any `agent_id` in a request body becomes untrusted input: usable to name a
  *target*, never to assert the *actor*.
- `check_s1` compares connection-UID against resolved-target-UID. #3508 already
  made it resolve both sides at one point of comparison; this removes the
  remaining assumption that `ctx.agent_id` was trustworthy.

An inherited token is scoped to one agent, so the retro's fallback position —
*"at minimum scope the key so an inherited copy is not omnipotent"* — is
achieved as a side effect.

## 7. Performance

The predecessor design put a store read on the delivery path; #3520's first P1
was exactly that, and its fix (pinning) was a workaround for a self-inflicted
cost. Carrying identity removes the cost rather than caching it.

| Path | Predecessor | This design |
|---|---|---|
| Message delivery | store read per lookup, cached | **no lookup** — UID is the key |
| Registration | store read per registration (every turn) | **none** — UID arrives in the payload |
| Work claim | string compare on slug | UID equality on an indexed column |
| Authz | resolve both sides | token→UID map hit, then UID equality |
| Name → UID | — | store read, but only when a human typed a name |

Net: the hot paths lose a database read they never should have had. The only
reads are on the cold, human-initiated path, where an extra millisecond is
irrelevant and correctness is everything.

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

- **M1 — Mint and carry.** Add `AGENTMUX_AGENT_UID` to pane env; add UID
  columns alongside slug columns; dual-write everywhere. **No reader changes.**
  Zero behaviour change; fully revertible by ignoring the new fields.
- **M2 — Registration.** Frontend and `input.rs` send the UID; the registry
  keys by it. Slug fallback retained **with a counter**, so §9.2 can measure
  whether anything still arrives without a UID.
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
