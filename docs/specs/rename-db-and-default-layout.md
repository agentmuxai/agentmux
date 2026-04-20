# Spec: Rename wave.db → objects.db + Default 2-column launch layout

**Status:** Draft
**Author:** AgentA
**Date:** 2026-04-20
**Scope:** Two unrelated-but-bundled housekeeping changes to ship together because they both touch first-launch state.

---

## Part 1 — Rename `wave.db` → `objects.db`

### Motivation

Two overlapping reasons:

1. **Functional naming.** `wave.db` is a brand leftover from the pre-rebrand Wave Terminal lineage and tells a reader nothing about what's inside. The contents are the domain object store — blocks, tabs, windows, workspaces, layouts, clients, agents — addressed through RPCs like `GetObject`/`UpdateObject`/`UpdateObjectMeta`, backed by the Rust `WaveStore` type. The natural functional name is `objects.db`. It pairs symmetrically with the sibling `filestore.db` (which holds blobs — terminal scrollback, agent doc cache), so the two filenames now tell you the storage model at a glance.

2. **Rebrand cleanup.** Every other user-visible surface was renamed to AgentMux (process names, env vars `AGENTMUX_*`, data dir `~/.agentmux/`, CLI scripts). The DB filename was the last stop-out. `objects.db` also drops the `wave-` prefix without replacing it with a redundant `agentmux-` — the containing directory (`~/.agentmux/db/` or `<portable>/data/db/`) already brands it.

Backward compatibility is **explicitly out of scope** — we don't migrate existing DBs. Any existing install keeps running fine until it's relaunched, at which point the new code creates a fresh empty `objects.db`. Users who want to preserve state rename the file manually. This is acceptable because:

- The product is still 0.33.x pre-release.
- Portable builds get a fresh DB per extracted folder anyway.
- The in-memory test helpers don't care.

### Related rename

`filestore.db` stays as `filestore.db` for now — it's already neutrally named. Re-evaluate only if we find another caller referencing it in a user-facing surface.

### Concrete changes

| File | Line(s) | Current | New |
|---|---|---|---|
| `agentmux-srv/src/main.rs` | 297 | `db_dir.join("wave.db")` | `db_dir.join("objects.db")` |
| `agentmux-srv/src/backend/blockcontroller/session_recovery.rs` | 134 | `tmp.path().join("wave.db")` | `tmp.path().join("objects.db")` |
| `agentmux-srv/src/backend/session_archive.rs` | 494 | `db_dir.join("wave.db")` | `db_dir.join("objects.db")` |

### Docs that mention `wave.db` (informational updates, non-breaking)

- `docs/analysis/offline-crash.md` (lines 105, 186) — update prose to match new name.
- `docs/analysis/blank-panes-orphaned-layout-nodes.md` (lines 4, 64) — update sqlite paths in examples.
- `docs/specs/portable-data-dir.md` (line 34) — `wave.db` → `objects.db` in the directory layout diagram.
- `docs/specs/SPEC_LAYOUT_HEAL_ROOTNODE_ORPHAN.md` (lines 22, 202) — same.
- `specs/runtime-logging.md` (line 271) — `db_path` example line.
- `specs/archive/go-to-rust-backend-port.md` — **archive**, no changes needed.

### Symbol renames (optional, not blocking)

`WAVE_DB_DIR`, `get_wave_db_dir`, `ensure_wave_db_dir`, `WaveStore`, `WAVE_DATA_HOME_ENV`, etc. are internal identifiers — they don't affect the user. A broader sweep is a separate PR. **This PR only changes the filename on disk.**

### Test plan

- [ ] Fresh portable extraction → launch → `<portable>/data/db/objects.db` exists.
- [ ] No `wave.db` file is created anywhere.
- [ ] Existing unit tests pass (`cargo test -p agentmux-srv`).
- [ ] Integration: create an agent, restart, agent still present → `objects.db` is being read, not a stale `wave.db`.

### Risks

- Anyone running dev mode against an existing `~/.agentmux/db/wave.db` loses state on first launch after this PR merges. Acceptable given the scope.
- If any script outside the repo (ops tooling, backup scripts, user docs off-repo) refers to `wave.db`, it breaks. Mention in the release note.

---

## Part 2 — Default launch layout

### Motivation

A fresh portable currently opens to an empty single-pane shell. First-time users have to manually split + pick widgets to get a useful workspace. Ship an opinionated default that demonstrates the core tripod — agent + swarm + system metrics — in one glance.

### Target layout

```
┌────────────────┬──────────────┐
│                │  CPU (20%)   │  ← sysinfo view
│                ├──────────────┤
│  Agent (tall)  │              │
│                │              │
│                │ Swarm (80%)  │
│                │              │
│                │              │
└────────────────┴──────────────┘
```

- **Root split:** horizontal (row), 50/50 — or possibly 60/40 left-biased; pick a ratio that reads well on 1280×800. Default row split is fine.
- **Left column:** single block, full-height `agent` view. No agent loaded — the picker renders so the user chooses (agentx / agenty / agentz / etc.).
- **Right column:** vertical (column) split, 20% top / 80% bottom.
  - **Top (20%):** `sysinfo` view (CPU plot).
  - **Bottom (80%):** `swarm` view.

### Implementation path

The backend seeds the initial workspace/tab/layout in `agentmux-srv`'s first-launch bootstrap. Grep for where the default window/tab is created (likely `backend/storage/wstore.rs` or a dedicated seed helper alongside `forge-seed.json`). The frontend layout reducer already handles arbitrary nested row/column splits — we just need to insert the right tree.

Blocks needed at seed time:
1. BlockDef `{ meta: { view: "agent" } }` — picker defaults
2. BlockDef `{ meta: { view: "sysinfo" } }`
3. BlockDef `{ meta: { view: "swarm" } }`

LayoutState tree:
```
root (row)
├── agent-block         size=10
└── child (column)
    ├── sysinfo-block   size=2
    └── swarm-block     size=8
```

(`size` units are relative; 10 on left + 10 total on right = 50/50 root; 2/8 inside right = 20/80.)

### Gating

Only seed this layout when the initial workspace has **zero** blocks. Existing workspaces (e.g. carried over via copied `data/db/`) must NOT be overwritten.

### Test plan

- [ ] Fresh portable: verify the 3-block layout matches the diagram above.
- [ ] Existing portable with pre-existing blocks: verify layout is untouched.
- [ ] Agent picker renders in the left pane (no agent loaded yet).
- [ ] Sysinfo shows the CPU plot.
- [ ] Swarm renders without error.
- [ ] Resize handles work on both the root split and the nested vertical split.

### Non-goals

- Customising the default via settings — out of scope; hardcoded for now.
- Remembering user's last layout across fresh extracts — out of scope; each extracted portable remains independent.
- Populating the agent with a default agentx/y/z — the picker is shown empty.

---

## Rollout

Single PR, single version bump. Title: `feat: rename wave.db → objects.db + default 2-column launch layout`. Merge after reagent approval. Build portable to verify both changes end-to-end.

## Future / deferred

A follow-up PR can rename the Rust identifiers to match:

- `WaveStore` → `ObjectStore`
- `wstore.rs` → `objectstore.rs` (or keep the file and just rename the type)
- `WAVE_DB_DIR` → `OBJECTS_DB_DIR` (or drop the global; the db path is computed anyway)
- `get_wave_db_dir` / `ensure_wave_db_dir` → `get_db_dir` / `ensure_db_dir`
- `WAVE_DATA_HOME_ENV` — already the string `"AGENTMUX_DATA_HOME"` under the hood; rename the Rust const to `DATA_HOME_ENV`.

That PR is identifier-only, mechanical, touches dozens of files, and benefits from a separate review so the filename change here doesn't get lost in the diff.
