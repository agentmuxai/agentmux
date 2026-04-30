# Phase E Multi-Reducer Migration — Status & Design Report

**Date:** 2026-04-30
**Branch state:** `main` at `e716a97a`
**Phase E PRs open:** none (loop paused)

---

## 1. Where we are

Three reducer-arm PRs shipped this session. All squash-merged.

| PR | Sub-phase | Scope | Rounds |
|---|---|---|---|
| [#611](https://github.com/agentmuxai/agentmux/pull/611) | E.2 | Workspace arms (Create/Delete) + SQLite bootstrap | 5 |
| [#612](https://github.com/agentmuxai/agentmux/pull/612) | E.2b | Tab arms + ActiveTab + workspace→tabs cascade | 2 |
| [#613](https://github.com/agentmuxai/agentmux/pull/613) | E.3 | Block arms + workspace→tabs→blocks cascade | 2 |

**Pattern that emerged:** ship reducer arms in tight focused PRs; defer the
"make-it-authoritative" plumbing (persist subscriber, RPC migration, host
bridge, renderer dispatcher) to a single consolidated phase (E.2c). This
discipline kept individual PRs <800 LoC and review rounds bounded.

**Reducer is currently a session-only projection:**
- Bootstrap loads from SQLite at startup.
- Pipe-originated commands (none in production yet — saga coordinator empty)
  mutate reducer state but NOT SQLite.
- HTTP/WS RPC continues writing to SQLite via wcore (authoritative).
- Process restart re-bootstraps from SQLite — divergence cleared.

This means the reducer's pipe-command path is dead-code in production today.
That's intentional: arms first, plumbing once it's well-designed.

**State now tracked in the reducer:**
- `WorkspaceRecord` { workspace_id, name, tab_ids, active_tab_id }
- `TabRecord` { tab_id, workspace_id, name, block_ids }
- `BlockRecord` { block_id, tab_id }

---

## 2. What's remaining

| Sub-phase | Scope | Status |
|---|---|---|
| **E.4** | Layout state arms | ⛔ design question (granularity) |
| **E.2c** | Persist subscriber + RPC migration + host bridge + renderer | ⛔ design questions (subscriber correctness, migration model) |
| **E.5** | Drag/tear-off sagas — first concrete saga consumers | depends on E.2c |
| **E.6** | Renderer multi-source consumption + saga buffering | depends on E.2c host bridge |
| **E.7** | Property tests + integration tests + `--diag` Tools | exit phase |

Both E.4 and E.2c need design input before implementation. E.5/E.6/E.7 are
downstream of E.2c.

---

## 3. Open design decisions

### 3.1 E.2c.1 — Persist subscriber bus-lag/HWM correctness

**The problem:** `tokio::sync::broadcast::Receiver::recv()` returns `Lagged(n)`
when a slow consumer falls behind capacity. The persist subscriber needs to
keep SQLite consistent with the reducer's emitted-event stream. If the
subscriber blindly advances its HWM past dropped events, those events never
reach SQLite — permanent divergence. Codex flagged this in E.2 review and
the subscriber was descoped to defer to E.2c.

#### Option A — Full reducer-state snapshot resync on `Lagged`

On `Lagged(n)`: lock the reducer's state, snapshot all workspaces/tabs/blocks
to SQLite via idempotent inserts + delete-not-in-snapshot, advance HWM to
the current `event_version`.

- **Pros**
  - Correct by construction. Handles arbitrary lag, including pathological cases.
  - State machine is simple: one event → one apply, OR one resync → one batch write.
  - Reuses bootstrap code path (resync ≈ bootstrap from in-memory state).
  - Reducer state in E.2/E.2b/E.3 is small (few KB) — snapshot is cheap.
- **Cons**
  - Lock contention spike on resync (reducer can't dispatch during snapshot).
    Bounded by state size; sub-millisecond at current scale.
  - SQLite write storm under sustained lag — but only on the slow path.
  - Doesn't scale gracefully to layout/block-content where state could grow large.

#### Option B — Larger broadcast capacity

Bump `tokio::broadcast::channel(N)` capacity from 1024 to 16384 (or higher).

- **Pros**: trivial change; defers the problem.
- **Cons**: doesn't fix it; just makes the failure rarer; loses correctness under
  sustained pressure (e.g., subscriber paused due to SQLite I/O hitch).

#### Option C — Per-event ACK + retry channel

Subscriber holds a sequence-number tracker. On `Lagged`, request retransmission
of `[hwm, latest_version]` from a side channel.

- **Pros**: granular recovery; minimal lock impact.
- **Cons**: requires retransmit channel + ring buffer in the publisher;
  complex protocol; significantly more code; redundant with the in-memory
  event log we already maintain (`agentmux-srv/src/event_log.rs`).

#### Option D — Freeze HWM on lag (codex called insufficient)

Track `lagged_since_start: bool`; freeze HWM forever once lagged.

- **Pros**: simple.
- **Cons**: permanent SQLite divergence after first lag; needs manual recovery
  on next process restart; codex's verdict on the original E.2 attempt at this.

#### Recommendation

**Option A (full resync).** The bounded-state argument carries the day at
current scale. If layout state grows large in E.4+, we can refine to Option C
without rewriting the rest. Reuses bootstrap logic. Provably correct.

The in-memory event log (`agentmux-srv/src/event_log.rs`, 4096-event ring)
gives us a future-Option-C path: subscriber could read replay slices from
the log instead of doing full snapshot resync. Defer that optimization until
profiled need.

---

### 3.2 E.2c.2 — RPC migration concurrency

HTTP/WS handlers (`dispatch_service::workspace.CreateWorkspace`,
`dispatch_service::tab.CreateTab`, etc.) currently call `wcore::create_workspace`
directly. Migration moves them to send `Command::CreateWorkspace` over the
srv pipe and await the corresponding event.

#### Option A — Big-bang cutover

One PR rewrites all dispatch handlers to route through reducer.

- **Pros**: atomic; no transitional dual-write; clean semantics.
- **Cons**: large PR (~600 LoC); rollback rolls back subscriber too.

#### Option B — Dual-write transitional

Both wcore and reducer pipe-command paths active during a stabilization window.

- **Pros**: safer rollback.
- **Cons**: race conditions on the same workspace/tab/block; lifetime
  questions ("when does dual-write end?"); diagnostic confusion.

#### Option C — Per-entity staged migration

Workspace first, then tab, then block — each its own PR.

- **Pros**: bounded blast radius per PR; gradual confidence build.
- **Cons**: mixed reducer-vs-wcore state during transition; subscribers see
  partial graph for the in-flight phase.

#### Recommendation

**Option A (big-bang) for workspace + tab + block in one PR.** Three entities
is small enough; the rewrite is mechanical (call-site replacement); atomic
cutover keeps semantics clean.

---

### 3.3 E.2c.3 — Host bridge subscription model

The host process needs to subscribe to srv events to forward them to renderers
via the existing CEF JS bridge.

#### Option A — Single persistent connection + reconnect

Host opens one srv-pipe connection at startup, holds for the host's lifetime.
On disconnect: reconnect with exponential backoff, request `GetSrvSnapshot`
+ `GetEvents { since: last_seen_version }` to catch up.

- **Pros**: matches existing launcher↔srv pattern (B.3+); one source-of-truth
  connection; well-understood reconnect semantics.
- **Cons**: host needs to track `last_seen_version`; reconnect-during-event
  race is a known pattern but still requires care.

#### Option B — Per-event re-connect

Open a fresh pipe connection per event.

- **Pros**: stateless.
- **Cons**: pipe handshake cost per event; obviously wrong at any non-trivial
  event rate.

#### Recommendation

**Option A.** Mirrors B.3 launcher↔srv connection lifecycle. The version-tracking
state is small (one u64 per pipe connection). Snapshot+resync on reconnect is
the same pattern subscribers will need anyway.

---

### 3.4 E.2c.4 — Renderer source-tagging

Spec §4.2.2: renderer dispatcher routes events to launcher-state vs srv-state
stores. Two recommended approaches.

#### Option A — Implicit (variant-name based)

`Event::WorkspaceCreated` → srv-source; `Event::WindowOpened` → launcher-source.
Source is a function of the variant.

- **Pros**: zero wire-format change; matches the convention since B.3;
  `frontend/util/launcher-events.ts` already uses this pattern.
- **Cons**: convention-dependent; future ambiguous variants would need a
  retroactive fix.

#### Option B — Explicit `source: ReducerSource` field

Add a `source` field to every `Event` variant.

- **Pros**: self-describing; eliminates convention dependency.
- **Cons**: wire-format expansion; touches every existing variant; needs
  serde forward-compat; cosmetic verbosity.

#### Recommendation

**Option A (implicit).** Matches spec recommendation. If a future variant
becomes ambiguous, switch at that point. No retrofit needed today.

---

### 3.5 E.4 — Layout state granularity

Persistent `LayoutState` has six fields:

```rust
pub struct LayoutState {
    pub oid: String,
    pub version: i64,
    pub rootnode: Option<serde_json::Value>,         // tree structure
    pub magnifiednodeid: String,
    pub focusednodeid: String,
    pub leaforder: Option<Vec<LeafOrderEntry>>,
    pub pendingbackendactions: Option<Vec<LayoutActionData>>,
    pub meta: Option<MetaMapType>,
}
```

Tab references it via `Tab.layoutstate: String` (the layout's oid).

The question: how does the reducer represent layout? Spec says "reducer ingests
it as commands."

#### Option A — One big `UpdateLayout { layout_id, layout_state: serde_json::Value }`

Reducer just records the latest opaque blob.

- **Pros**: trivial reducer arm; no tree-walk logic.
- **Cons**: reducer becomes an opaque-blob shipper, NOT canonical for layout.
  Renderer can't react granularly. Loses "reducer is the source of truth"
  property. Spec's "ingest as commands" intent violated.

#### Option B — Full granular ops

Commands: `SplitNode`, `MergeNodes`, `FocusNode`, `MagnifyNode`, `ReorderLeaves`,
`AddPendingAction`, `RemovePendingAction`. Events for each.

- **Pros**: reducer truly understands layout; granular renderer reactions;
  testable invariants ("focused node always exists in tree").
- **Cons**: ~10 commands and events; tree-mutation logic in pure-functional
  reducer is awkward (rootnode is `serde_json::Value`, not a typed tree);
  large PR; LayoutActionData semantics need translation.

#### Option C — Minimal slice (focus + magnify only)

Commands: `SetFocusedNode`, `SetMagnifiedNode`. Events: `FocusedNodeChanged`,
`MagnifiedNodeChanged`. `LayoutRecord` tracks `{ layout_id, tab_id,
focused_node_id, magnified_node_id }`. The full rootnode tree, leaforder,
and pending actions stay in SQLite via wcore (untouched).

- **Pros**: same scope-reduction discipline as E.2/E.2b/E.3; ~250 LoC PR;
  immediate test surface; covers the most-touched layout state (focus changes
  every click).
- **Cons**: reducer has incomplete view of layout (no tree, no leaforder).
  A subscriber wanting "current layout state" must combine reducer (focus)
  with wcore (tree).

#### Option D — Skip E.4 for now; do E.2c first

Land E.2c so the reducer is authoritative for what it already tracks
(workspace/tab/block lifecycle), then come back to E.4 once the
authoritative-state pattern is proven.

- **Pros**: avoids opening another design front; E.5 sagas can begin without
  layout being in the reducer (drag/tear-off sagas operate on tabs and blocks,
  not layout nodes).
- **Cons**: defers E.4; spec ordering not honored.

#### Recommendation

**Option C as E.4** if the goal is more momentum on reducer arms.
**Option D (skip)** if the goal is to consolidate around the authoritative-
state model first — which is probably the better engineering call given the
unresolved E.2c design questions.

My slight preference: **Option D**. Land E.2c first (it's the linchpin),
then revisit E.4 with the authoritative-state pattern in hand.

---

## 4. Carryovers

### From #613 (codex P2)

**Ambiguous block-parent during bootstrap.** When persisted SQLite contains the
same `blockid` in multiple `Tab.blockids` lists (corrupt state), the reducer's
reverse-lookup picks one tab non-deterministically (`HashMap::values().find(...)`).

**Defensive-repair fix for the next persist.rs PR:** detect multi-parent
references explicitly, log a warning, skip the block (or pick deterministically
by smallest tab oid). Bootstrap is already doing other defensive repair (orphan
tab/block dropping); this fits naturally.

---

## 5. Recommended sequencing for next session

```
E.2c.1  Persist subscriber                                ~300 LoC, 1 PR
        ↓ (Option A: full-resync-on-lag)
E.2c.2  RPC migration through reducer                     ~400 LoC, 1 PR
        ↓ (Option A: big-bang for workspace+tab+block)
E.2c.3  Host bridge + renderer dispatcher                 ~500 LoC, 1 PR
        ↓ (Option A: persistent connection; Option A: implicit source)
E.4     Layout state                                      ~300 LoC, 1 PR
        ↓ (Option C: minimal slice)
E.5     Drag/tear-off sagas                               ~700 LoC, 1 PR
E.6     Renderer multi-source + saga buffer               ~600 LoC, 1 PR
E.7     Property tests + integration tests + --diag       ~500 LoC, 1 PR
```

Total: ~3300 LoC across 7 PRs. With ~2-4 review rounds per PR (based on
observed cadence), expect ~20-30 review cycles total before E.7 merges.

---

## 6. What I'd want from the user before resuming

For E.2c (the next PR):

1. **Subscriber correctness:** confirm Option A (full-resync-on-lag) or pick
   alternative.
2. **RPC migration:** confirm Option A (big-bang for workspace+tab+block) or
   pick alternative.
3. **Host bridge:** confirm Option A (persistent connection + reconnect) or
   alternative.
4. **Source tagging:** confirm Option A (implicit) or alternative.

A short answer like "go A,A,A,A on E.2c" is enough to resume. Or
"skip E.4, do E.2c.1 first" if you want to break it up further.

---

## 7. Process notes

The 5-round review on E.2 was almost entirely reagent flagging stale phase-
boundary comments after each refactor (rounds 3-5 were just doc-string fixes).
For future phases:

- **Run a grep sweep before pushing** for stale "E.{X} adds Y" / "no Z yet" /
  "subscriber" / "skeleton" framing. Cheaper than 3 review rounds.
- **Treat reagent's regression-detection as a tripwire**: when reagent says
  "Merge Analysis: 1 regression(s) detected", that's a structural concern,
  not a doc nit — read it before any push.

The 2-round cadence on E.2b and E.3 reflects this lesson being internalized.
