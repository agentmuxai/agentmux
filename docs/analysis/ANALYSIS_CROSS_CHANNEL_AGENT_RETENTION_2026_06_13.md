# Analysis — Cross-channel / cross-version agent retention is broken

**Date:** 2026-06-13
**Goal:** custom agents created in any channel/version should appear in **every** channel/version (My Agents), and survive upgrades.
**Status:** Root-caused. The cross-channel P0 work (#1383–#1390) ships, but the **migration that backfills existing agents into the global store is broken three ways**, so almost no agents become cross-channel. The read/merge path itself is sound.

---

## TL;DR

The global cross-channel store (`~/.agentmux/shared/agents/definitions/`) is supposed to hold **all** custom agent definitions. In practice it holds **exactly one** ("Mazs"), so a fresh channel/version (e.g. a new `task package` portable on channel `local-main`) shows only the 7 seeded built-in templates and none of your real agents (Qooma, Clamk, Naki, CodexPo, GeminiOpp, …).

The one-shot backfill migration (`registry/def_migrate.rs`) fails to populate the global store because:

1. **Schema-fragile query** — it `SELECT`s `container_image`/`container_volumes`/`container_name`; **older channel DBs don't have those columns**, so SQLite returns *"no such column: container_image"* and the migration **skips the entire DB**. Log from this machine: `dbs_scanned=12 dbs_skipped=10 rows_seen=1 records_written=1`. 10 of 12 DBs skipped → your agents are in the skipped DBs.
2. **`dev/` is never scanned** — it only walks `channels/*/versions/*`; agents in `dev/<branch>/…` (e.g. **Qooma** in `dev/main`) are invisible to it.
3. **Idempotent marker written after that incomplete run** — `.migrated_definitions` is written unconditionally, so the migration **never runs again** — not even after the DBs are upgraded to the new schema or new channels appear.

The read/merge path is **correct** and the store attaches at the right path (logs confirm), so once the global store is actually populated, agents will surface.

---

## Evidence

### Data state (this machine)
| Where | Custom agents |
|---|---|
| `dev/main/.../objects.db` | **Qooma** (+ 7 built-ins) |
| `channels/local-agentx-fix-term-scrollbar-…/…/objects.db` | **Clamk, Naki, CodexPo, GeminiOpp** (+ 7) |
| `channels/local-main-b28b7a/.../objects.db` (fresh portable) | none — 7 seeded built-ins only |
| **`~/.agentmux/shared/agents/definitions/`** (global) | **only `Mazs`** + `.migrated_definitions` marker |

### Migration log (the smoking gun) — `target=agentmux_srv::registry::def_migrate`
```
WARN def migrate: skipping unreadable/incompatible objects.db
     db=…\channels\verify2\…\objects.db
     error="no such column: container_image ... FROM db_agent_definitions WHERE is_seeded = 0"
     (×10 — verify2..verify6 and others)
INFO def registry: global definition migration finished
     dbs_scanned=12 dbs_skipped=10 rows_seen=1 records_written=1
INFO def registry: global definition store attached  dir=…\.agentmux\shared\agents\definitions
```
→ 10/12 DBs skipped on a missing column; only 1 user agent ever seen; store attaches fine (read path OK).

---

## Architecture (intended) — spec `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`

- **Per-channel SQLite** `…/data/db/objects.db` (`db_agent_definitions` / `db_agents`) — the channel's templates + its own user agents.
- **Global file store** `~/.agentmux/shared/agents/definitions/<id>.json` — the cross-channel roster, resolved by `resolve_shared_definitions_dir()` (`registry/paths.rs:37`) → `resolve_global_shared_root()` (`paths.rs:42`, the true `~/.agentmux/shared`, decoupled from `AGENTMUX_DATA_DIR`).
- **Populated by:** (a) one-shot backfill migration (`def_migrate.rs`), (b) live write-mirror on create/edit/delete (`def_registry_mirror.rs`, P0.2b).
- **Read by:** `Store::agent_def_list()` (`storage/agents.rs:364`) — local SQLite, then overlay the global roster.

---

## Root causes (confirmed)

### RC1 — Migration skips whole DBs on a missing column *(primary)*
`registry/def_migrate.rs:131-142` builds a fixed `SELECT … container_image, container_volumes, container_name … FROM db_agent_definitions`. Older DBs predate those columns, so `query_map` errors with `no such column: container_image`. That error is **not** a "missing table", so `tolerate_missing_table` (`def_migrate.rs:213`, which only swallows `no such table`) does **not** catch it — it propagates out of `read_user_definitions`, the DB is counted `dbs_skipped`, and **every agent in that DB is lost to the migration** (`def_migrate.rs:84-91`). On this machine that's 10 of 12 channel DBs.

### RC2 — Migration never scans `dev/`
`def_migrate.rs:55-94` walks only `home/channels/*/versions/*/data/db/objects.db`. The **instance** migration does it right — `registry/migrate.rs:330-372` `enumerate_sources()` scans `channels/*` **and** `collect_dev_sources(home/dev)` — but the **definition** migration has no equivalent. So `dev/<branch>/…` agents (Qooma in `dev/main`) are never seen.

### RC3 — One-shot marker poisons recovery
`def_migrate.rs:50-52` early-returns if `.migrated_definitions` exists, and `:122-125` writes it **unconditionally** after the pass — even when 10/12 DBs were skipped. So after the broken first run, the migration is permanently disabled. The header comment's rationale ("the live write-mirror backfills any skipped agent on its next edit") **fails for agents that are never edited again** — which is most of them. Once RC1/RC2 are fixed, the marker still blocks the re-scan that would recover everything.

### Read path — NOT a root cause (verified)
`agent_def_list()` (`storage/agents.rs:391-406`) correctly **adds** global-only records (`by_id.insert(def.id…)` at :405), the RPC filter (`server/agent_handlers.rs:117-127`) passes `is_seeded=0 / user_hidden=0` rows (Mazs qualifies), and the store attaches at the correct dir per the log. So global definitions *do* surface once present. (The earlier "Mazs doesn't appear" observation is secondary — with only Mazs global it's a thin signal; worth a quick live re-check after RC1–RC3 are fixed and the store is repopulated, candidate edge being `def_store::list_active` validation/`retired` handling, `def_store.rs:46-90`.)

---

## Why this misses the goal

Spec §9 **AC5 — "first launch after upgrade migrates every channel's per-version agents into the global roster."** Violated by RC1 (skips old-schema DBs), RC2 (skips `dev/`), and RC3 (never retries). The feature's *steady-state* (write-mirror on edit) works, but **retention of existing agents** — the whole point for current users — does not.

---

## Fix proposals

### F1 — Make the migration schema-resilient (fixes RC1)
Don't hard-require columns that old DBs lack. Options, best first:
- **Introspect columns** via `PRAGMA table_info(db_agent_definitions)` and build the `SELECT` from the intersection of wanted ∧ present columns; default the rest (`container_image=""`, `container_volumes="[]"`, …). Robust to any schema drift.
- Or **treat `no such column` like `no such table`** — extend the tolerated-error set so a missing optional column degrades to defaults instead of skipping the whole DB. (`is_missing_table`/`tolerate_missing_table`, `def_migrate.rs:209-219`.)

### F2 — Scan `dev/` too (fixes RC2)
After the `channels/*` loop, also enumerate `home/dev/<branch>[/<sub>]/data/db/objects.db` — reuse `migrate.rs::collect_dev_sources` (or factor a shared source-enumerator so the definition and instance migrations can't diverge again).

### F3 — Make the marker recoverable (fixes RC3)
The one-shot model is the trap. Replace it with one of:
- **Versioned marker** — store a migration *version* (e.g. `{"v":2}`); re-run when the binary's migration version is newer than the marker. Bump the version whenever the scan logic changes (so this fix re-runs once for existing users).
- **Only mark complete on a clean pass** — record `dbs_skipped`; if any DB was skipped, don't write the terminal marker (write a "partial" marker and retry next launch, with backoff so a permanently-broken DB doesn't loop forever).
- **Drop the marker; make re-scan cheap + idempotent** — `upsert` already no-ops on existing/tombstoned ids; a periodic/opportunistic re-scan of changed DBs (mtime-gated) keeps the roster complete without a one-shot.

### F4 — Immediate recovery for existing users
Ship F1+F2+F3, and on upgrade **invalidate the stale marker** (delete `.migrated_definitions`, or supersede it via the versioned marker) so the fixed migration re-runs once and backfills every now-readable DB (channels + dev). One-time, idempotent, global.

### F5 — Verify the read surface live
After repopulation, confirm a global-only definition appears in My Agents across a *fresh* channel (the Mazs case). The code path is correct; this is a regression guard, not a known bug.

### F6 (strategic) — finish the rollout (P1/P2)
The migration is fragile precisely because the global store is *secondary* to per-channel SQLite. The spec's **P1 (registry-primary dual-write)** and **P2 (cutover)** make the global store authoritative, shrinking reliance on a one-shot backfill. Worth prioritizing if cross-channel retention is a headline feature.

---

## Recommended sequence
1. **F1 + F2** — the migration actually reads every DB (channels + dev) regardless of schema age.
2. **F3 + F4** — make it re-runnable and invalidate the poisoned marker so existing users recover on next launch.
3. **F5** — live-verify the read surface.
4. Re-run on this machine and confirm Qooma/Clamk/Naki/CodexPo/GeminiOpp/Mazs all land in `~/.agentmux/shared/agents/definitions/` and appear in a fresh portable.
5. **F6** when ready — P1/P2 to make retention structural.

---

## Key files
- `agentmux-srv/src/registry/def_migrate.rs` — the broken backfill (`migrate_definitions_global_once` :45; fixed `SELECT` :134-142; `channels`-only walk :55-94; tolerate-only-missing-table :209-219; unconditional marker :122-125; early-return :50-52).
- `agentmux-srv/src/registry/migrate.rs:330-372` — instance migration that *does* scan `dev/` (`collect_dev_sources`) — the template for F2.
- `agentmux-srv/src/backend/storage/agents.rs:364-415` — `agent_def_list` global-merge (correct).
- `agentmux-srv/src/backend/storage/def_registry_mirror.rs` — live write-mirror (P0.2b).
- `agentmux-srv/src/registry/def_store.rs:46-130` — global file store (open/list_active/retire).
- `agentmux-srv/src/registry/paths.rs:37-56` — `resolve_shared_definitions_dir` / `resolve_global_shared_root`.
- `agentmux-srv/src/server/agent_handlers.rs:117-133` — My Agents RPC filter (correct).
- `agentmux-srv/src/main.rs:~502` — startup wiring (`migrate_definitions_global_once` + store attach).
- Spec: `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` (§9 AC5).

*Written 2026-06-13 by AgentX. Analysis only — no code changed.*
