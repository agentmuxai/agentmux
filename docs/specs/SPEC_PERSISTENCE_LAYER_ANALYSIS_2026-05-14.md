# SPEC: Persistence layer analysis — keep SQLite, or move?

**Date:** 2026-05-14
**Author:** AgentX
**Status:** Analysis (no decision yet)
**Triggered by:** the upcoming `Wave*` → `Mux*` rename PR includes a SQLite schema migration (`db_wave_file` → `db_mux_file`). Before paying that migration cost, sanity-check whether the persistence layer itself is the right shape.

---

## 1. TL;DR

**Keep SQLite for now.** The architecture is a good fit for what AgentMux actually does, and switching costs strictly exceed switching benefits today. Three caveats where the answer flips:

1. If **cross-instance shared state** becomes a goal, an event-log-based store starts to look better than SQLite (which the team has already rejected for that case in the agent-registry spec).
2. If the team wants to **drop the C dependency** for principled "Rust-everywhere" reasons, an embedded Rust KV (`redb` / `fjall` / `sled`) is mature enough.
3. If **simplification** is the goal more than correctness, JSON-per-object files (like `agents/` already are) reduce moving parts but lose atomicity across objects.

The schema rename in the upcoming PR is a single `ALTER TABLE` — not a meaningful reason to revisit persistence. Defer this decision.

---

## 2. What we use SQLite for today

Three SQLite databases, all in the per-version data dir, all single-writer (each has its own `Mutex<Connection>`):

| Store | Tables | What's in it |
|---|---|---|
| **`objects.db`** (`WaveStore`) | 7 core (`db_client`, `db_window`, `db_workspace`, `db_tab`, `db_layout`, `db_block`, `db_temp`) + Forge v1-v9 extensions (~18 more) | App-domain reducer state. Every persisted object is `(oid TEXT PK, version INTEGER, data TEXT)` where `data` is a serde-JSON blob of the full object |
| **`filestore.db`** | `db_wave_file` (metadata), `db_file_data` (64 KB BLOB chunks) | Forge configs, snapshots, screenshots, file pane buffers |
| **`sagas.db`** | `saga`, `saga_step` | Cross-block side-effect lifecycle (resume on crash) |

Plus `launcher-sagas.db` (same shape, owned by launcher).

### 2.1 What we use SQLite *for*

- **Single-key `WHERE oid = ?1` lookups.** Every production query.
- **Atomic transactions** for multi-row mutations (`WaveStore::with_tx`).
- **FK cascade deletes** in Forge tables (v6+).
- **WAL mode** for crash safety + multi-reader concurrency within an instance.
- **Bulk read at boot** (`bootstrap_state_from_wstore` loads everything into in-memory reducer; rest of the session reads from RAM).

### 2.2 What we DON'T use SQLite for

- **No JSON1** (`json_extract`, `json_set`) — `data` columns are opaque blobs, deserialized in Rust.
- **No FTS5** — no full-text search anywhere.
- **No window functions, CTEs, triggers, views.**
- **No JOINs in production code.** Every query is single-table key access.
- **No ad-hoc query interface.** Application code knows the shape; the DB is a typed key-value store dressed in SQL clothes.

### 2.3 How the choice was made

**Inherited from the Wave Terminal fork.** No design-rationale doc exists in `docs/specs/`. `agentmux-srv` is a Rust port of Wave's Go backend, which used SQLite for the same OID→serde(JSON) pattern. The choice carried over without being re-evaluated.

The team *has* re-evaluated SQLite for one specific case — the **shared agent registry** (cross-version, cross-instance shared state) explicitly rejected SQLite because:

- WAL/SHM lock contention between processes
- Schema migrations on a shared DB break older versions
- Per-file versioning is better for forward/backward compat

So there's awareness of SQLite's edges; the decision is "good enough for per-instance state, wrong for shared state."

---

## 3. The architectural fingerprint

What we *actually need* from a persistence layer:

| Property | Required |
|---|---|
| Key-value access by string OID | ✓ (every query) |
| Atomic durability per write (fsync correctness) | ✓ |
| Atomic *batched* writes (transaction across N keys) | ✓ (WaveStore::with_tx) |
| Crash-safe (WAL or equivalent) | ✓ |
| Single-writer-per-store within an instance | ✓ (current code; lock-serialized) |
| Multi-reader within an instance | ✓ (WAL gives this) |
| Range queries / scans / joins | ✗ (no production use) |
| Full-text search | ✗ |
| Ad-hoc query language | ✗ (no user-facing query surface) |
| Schema migrations between versions | ✓ (Forge v1-v9 evolution proves it works) |
| Cross-instance / cross-process sharing | ✗ for objects.db / sagas.db; ✓ aspirationally for shared agent registry (which uses files, not SQLite) |
| <500 MB working set per version | ✓ (typical) |
| <10 KB per object | ✓ (typical) |

**Read this as:** "We need a typed embedded KV store with transactions and crash safety." SQL is incidental — we never use it as a query language.

---

## 4. Alternatives, honestly evaluated

### 4.1 Stay with SQLite (status quo)

**Pros:**
- Zero migration cost.
- Mature, well-understood, predictable failure modes.
- WAL + busy_timeout + transactions all work the way you expect.
- `rusqlite` is a stable crate; bindings + tooling are excellent.
- **Schema migrations are a solved problem** — Forge has already done v1→v9 evolution successfully.
- `sqlite3` CLI lets us inspect data ad-hoc when debugging.

**Cons:**
- C dependency. SQLite is bundled (statically linked usually), so this is small but not zero — affects build complexity on cross-compile, and there's a compile-step dependency.
- SQL is overkill for what we use it for; the schema-as-DDL adds ceremony.
- Schema migrations require testing on existing DBs (real cost — see the upcoming `db_wave_file` rename PR).
- Multi-writer concurrency is awkward (you need a lock layer in app code; we already have one).

### 4.2 Embedded Rust KV — `redb` (recommended if we move)

[`redb`](https://github.com/cberner/redb) is a Rust-native B+tree KV with ACID transactions, MVCC, and zero-copy reads. Stable since 2.0.

**Pros:**
- **Zero C dependencies** — pure Rust, simpler builds.
- ACID transactions out of the box.
- Crash-safe (single-file, like SQLite).
- Type-safe schemas via Rust traits — no DDL strings.
- Schema migrations are application-managed (write a `read v1, write v2` migration in Rust).
- Faster than SQLite for keyed access (no SQL parser overhead).

**Cons:**
- **No SQL CLI for ad-hoc inspection.** You'd write a small Rust tool or a debug RPC. Tractable but a real friction.
- Younger ecosystem than SQLite; fewer Stack Overflow answers.
- File format is `redb`-specific — less portable than SQLite for backups/export.
- No `WHERE`-style filtering; if we ever need it, we'd have to scan.

**Verdict:** Solid second choice. Real win if "no C deps" matters; otherwise marginal.

### 4.3 Other embedded KV — `sled`, `fjall`, LMDB, RocksDB

- **`sled`** — pre-1.0, author has expressed concerns about long-term maintenance. Not recommended.
- **`fjall`** — newer LSM-tree, Rust-native. Promising but young.
- **LMDB** (`heed` crate) — battle-tested, very fast reads. C dependency, single-writer model is restrictive.
- **RocksDB** — operational-grade LSM, but a heavyweight C++ dependency. Wrong fit for an app store.

None of these is meaningfully better than `redb` for our shape.

### 4.4 JSON-files-per-object (file-based, no DB)

Like `agents/` already is. Each object is `<data-dir>/objects/<otype>/<oid>.json`.

**Pros:**
- Maximum simplicity. No DB engine. `cat`/`jq` to inspect.
- Easy to back up (just copy the dir).
- Easy to share/version (per-object Git history if wanted).
- **Already proven** for the agent-registry spec where SQLite was explicitly rejected for cross-version use.

**Cons:**
- **No atomic batched writes across objects.** Saving a workspace + 5 tabs in one transaction means writing 6 files; if you crash halfway, you have a partial state. The reducer's "all-or-nothing" guarantees are gone.
- Per-write cost is higher (one file open + fsync per object vs. one transaction commit for many).
- Listing operations require reading the directory — slower than indexed scans.
- Filename collisions, character escaping in OIDs, case-insensitive filesystem traps.

**Verdict:** Wrong for `objects.db` (atomicity matters), maybe right for `filestore.db` if BLOBs are big and rarely batched.

### 4.5 Append-only event log + in-memory snapshot (event sourcing)

The launcher already does this with `launcher-events.log` (JSONL, written via `event_log::run_disk_writer`). Extend the model: every state change is an event; current state is the in-memory replay; persistence is the log.

**Pros:**
- **Time-travel debugging is free.** Replay the log to any point in time.
- Crash safety is trivial — log append is atomic.
- Cross-instance / shared-state story is much cleaner (events can be merged across writers).
- Aligns with the existing reducer architecture.

**Cons:**
- **Snapshotting is required** — without it, every boot replays the entire history (slow with thousands of events).
- Log compaction is non-trivial (you can't delete old events without losing replayability).
- Migration story is harder — new event-shape parsers must handle every old event type forever.
- Filestore (BLOB content) doesn't fit the event-log model.
- Two-store architecture (events + snapshots) is more moving parts than one SQLite file.

**Verdict:** Best architectural fit for cross-instance shared state. Worse for the per-instance objects.db case where SQLite already works. Could be a Phase 2 if the cross-instance question gets revisited.

### 4.6 SurrealDB embedded / DuckDB / SurrealKV

- **SurrealDB embedded** — graph + document + KV. Massive surface area for what we need. Overkill.
- **DuckDB** — analytics OLAP, columnar. Wrong fit (we have <500 MB OLTP).
- **SurrealKV** — newer, less proven, Rust-native KV. Comparable to `redb` but younger.

None are recommended.

---

## 5. Comparison matrix

| | SQLite | redb | JSON files | Event log |
|---|---|---|---|---|
| C deps | Yes (small) | **No** | No | No |
| ACID transactions | ✓ | ✓ | ✗ (per-file only) | ✓ (append) |
| Crash safety | ✓ (WAL) | ✓ | ✓ (per-file) | ✓ |
| Multi-reader | ✓ (WAL) | ✓ | ✓ | ✓ |
| Cross-instance write | hard | hard | possible | natural |
| Time-travel debugging | hard | hard | hard | **trivial** |
| Ad-hoc inspection | `sqlite3` CLI | custom tool | `jq` | custom tool |
| Migration cost from today | 0 (status quo) | **2-4 weeks** | 1-2 weeks (lose atomicity) | 4-8 weeks |
| Engineering effort to build | already done | meaningful | moderate | substantial |
| Failure-mode familiarity | high | medium | high | low |
| Suits the fingerprint (§3) | well | well | poorly (atomicity) | well (with snapshots) |

---

## 6. Recommendation

**Keep SQLite. Defer this decision.**

The honest take: AgentMux's persistence shape is a perfect fit for either SQLite or `redb`. The only material differences are (a) `redb` removes a C dep, and (b) SQLite has better debugging tools. Neither is a "we must move" pressure.

What WOULD change the answer:

1. **A push to make state cross-instance shared by default** — the agent registry has already declined SQLite for this. If that pattern spreads, an event log starts to look better than SQLite for the new domain. (At which point: keep SQLite for objects.db, add an event log for the shared layer. Mixed-store, not migration.)

2. **A "Rust everywhere, no C" policy decision** — would push toward `redb`. Pure aesthetic / dep-management value; no functional improvement.

3. **The Forge schema getting unwieldy** — v1→v9 has been managed. If migrations become a regular pain point, a schema-less store (event log or JSON-per-object) might reduce friction. Not yet the case.

4. **Performance becoming a concern** — currently it isn't. Per the docs there are no benchmarks tracking persistence latency, which is itself a sign that nobody's noticed it as a problem.

**For the immediate `Wave*` → `Mux*` rename PR:** the schema rename is one `ALTER TABLE` statement against an already-versioned migration framework. It's not a reason to revisit the persistence layer. Ship the rename as planned; this analysis is "considered and explicitly deferred."

---

## 7. What this changes for the rename PR

**Nothing structural.** The rename PR proceeds as designed:

- `db_wave_file` → `db_mux_file` via `ALTER TABLE` migration
- `WaveStore` → `MuxStore` (type rename only, no storage change)
- All other renames as in `SPEC_WAVE_TO_MUX_RENAME_2026-05-14.md`

If the team later decides to migrate persistence (per §6 triggers), the new abstraction layer will inherit the `Mux*` naming cleanly.

---

## 8. If we *did* want to move — sketch of a phased migration

For completeness, if §6 ever points toward `redb`:

1. **Phase 1** — wrap the existing `WaveStore` API in a trait (`ObjectStore` with `get<T>`, `put<T>`, `with_tx`, etc.). One implementation: SQLite (existing). Tests are at the trait level.
2. **Phase 2** — add a second implementation: `redb`-backed `ObjectStore`. Run both side-by-side via dual-write, compare reads.
3. **Phase 3** — swap the default to `redb`; SQLite implementation stays for backups + migration source.
4. **Phase 4** — one-shot migration tool reads SQLite, writes `redb`, verifies, swaps.
5. **Phase 5** — drop SQLite implementation.

Estimated 2-4 weeks of focused work to do safely. Not justified today.

---

## 9. Open questions (none blocking)

1. Is there interest in cross-instance shared state beyond the agent registry? If yes, that pushes toward event-sourcing for the new layer (independent of objects.db).
2. Has anyone benchmarked SQLite's perf in AgentMux under heavy load (e.g. a swarm of 50 agents writing concurrently)? If not, the "no perf concerns" claim is by-vibe, not by-data.
3. Does the team have an opinion on the C-deps-vs-Rust-only tradeoff? If "drop C deps" becomes policy, the answer here flips toward `redb`.
