# SPEC: Data channels — version-spanning data isolation for AgentMux

**Date:** 2026-05-24
**Author:** AgentA (Claude Opus 4.7)
**Tracking discussion:** [#1026](https://github.com/agentmuxai/agentmux/discussions/1026)
**Research basis:** [`docs/research/RESEARCH_PER_VERSION_DATA_ISOLATION_2026_05_24.md`](../research/RESEARCH_PER_VERSION_DATA_ISOLATION_2026_05_24.md)
**Builds on:** [`SPEC_DATA_DIR_UNIFICATION_2026-05-05.md`](./archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md) (Phase 1 — per-version isolation; this is its Phase 2)

---

## TL;DR

Replace per-version data isolation (`~/.agentmux/versions/<version>/`) with **per-channel** isolation (`~/.agentmux/channels/<channel>/`). Within a channel, every version reads and writes the same data dir; agents, identity, memory, conversations all persist across patch/minor bumps. Across channels, full isolation — `stable` and `beta` and `local-<branch>-<hash>` are independent worlds.

Applies to **both Installed and Portable**. Each runtime mode resolves to a default channel:

| Runtime mode | Default channel | Built-by |
|---|---|---|
| Installed (production install) | `stable` | release CI |
| Portable (downloaded released ZIP) | `stable` | release CI |
| Portable (local `task package` build) | `local-<branch>-<hash>` | the operator |
| Dev (`task dev`) | `dev-<branch>` | `task dev` |

The user can override with `AGENTMUX_CHANNEL=<name>` for parallel-channel testing.

Forward-only schema migrations run on launch when a newer version opens a channel with an older schema. A safety lock refuses to open a channel whose schema is **newer** than the running binary (prevents corruption by downgrade). Snapshots auto-saved before any migration; auto-pruned (keep last 5).

Three shippable PRs, in order: A (channels structure) → B (migration framework + safety lock) → C (import wizard).

---

## 1. Current state (recap, for reference)

Per `SPEC_DATA_DIR_UNIFICATION_2026-05-05` Phase 1, shipped in PR #695:

```
~/.agentmux/
├── shared/                           ← account-wide (chromium-cookies, agent-cache, ...)
├── versions/<version>/               ← INSTALLED + PORTABLE — per-version
│   ├── data/                         ← objects.db, sagas.db, filestore.db
│   ├── agents/                       ← per-agent working dirs
│   ├── config/
│   ├── logs/
│   ├── cef-cache/
│   ├── runtime/                      ← ipc-port, lockfile, etc.
│   └── instance.json
└── dev/<branch>/                     ← DEV — per-branch
    └── ... (same shape as versions/<v>/)
```

Resolution: `agentmux_common::DataPaths::resolve(version, mode)` in `agentmux-common/src/data_paths.rs:81-117`.

**Pain point:** every `task package` bump produces a fresh, empty `versions/<new-version>/`. My Agents list resets. Conversation history is gone. To compare two builds you re-create the test agent each time. The 2026-05-05 spec flagged this explicitly (§2.6 line 103) as a known cost of the Phase 1 design.

---

## 2. Target design — channels

### 2.1 New layout

```
~/.agentmux/
├── shared/                           ← unchanged from Phase 1
├── channels/
│   ├── stable/                       ← ALL released versions (Installed + Portable-ZIP) land here
│   │   ├── data/
│   │   │   ├── objects.db
│   │   │   ├── sagas.db
│   │   │   ├── filestore.db
│   │   │   └── meta.json             ← {schema_version, channel_created_at, last_run_version}
│   │   ├── agents/
│   │   ├── config/
│   │   ├── logs/
│   │   ├── cef-cache/
│   │   └── runtime/                  ← single-instance lockfile (one process per channel)
│   ├── beta/                         ← reserved; not populated in Increment A
│   └── local-<branch>-<hash>/                 ← local `task package` builds — for personal smoke-test
├── dev/<branch>/                     ← unchanged from Phase 1 (kept as a fourth "channel namespace")
└── snapshots/                        ← auto-backups before migration (Increment B)
    └── stable-pre-v0.42.0-2026-06-15T13-22-08Z.bak/
```

**Why a separate `channels/` parent dir** (vs. mixing channels at root): keeps the "this is a channel" semantics visually obvious, and leaves room for non-channel system dirs (`shared/`, `snapshots/`, `dev/`) without naming collisions.

**Why `dev/<branch>/` stays outside `channels/`**: branches are short-lived and numerous; promoting them to first-class channels would clutter the namespace. They're already a different concept (per-branch isolation for parallel feature work) and the existing path is established.

### 2.2 Default channel per runtime mode

`agentmux_common::RuntimeMode` already distinguishes `Installed`, `Portable`, `Dev { branch }`. The new mapping:

| `RuntimeMode` | Default channel | Notes |
|---|---|---|
| `Installed` | `stable` | Production install — same channel across all installed versions |
| `Portable` AND built by release CI | `stable` | Released portable ZIP downloaded by a user |
| `Portable` AND built by `task package` locally | `local-<branch>-<hash>` | Differentiated via embedded build marker (see §2.5) |
| `Dev { branch }` | `dev-<branch>` (renamed for symmetry; resolves to `~/.agentmux/dev/<branch>/`, unchanged path on disk) | Per-branch — existing behavior |

The operator can override with `AGENTMUX_CHANNEL=<name>` to point an `Installed` / `Portable` binary at any channel (e.g., for testing a hot-fix build against the live stable data). **Dev mode does NOT honor the override** — the launcher and host both use `resolve_path_only` for dev builds, so a `task dev` session launched from inside a parent agentmux pane can't inherit the parent's channel and break per-branch isolation (codex P2 on PR #1027 caught this — without the symmetric ignore, launcher and host would disagree on the single-instance lock path). If you want a non-default channel, use a portable build.

### 2.3 Channel-name validation

Same rules as `sanitize_path_segment` in `agentmux-common/src/data_paths.rs`:
- Lowercase ASCII letters, digits, `-`, `_`
- No `.` or `..` traversal
- Length 1..64
- Rejects empty after sanitization

Reserved channel names (cannot be used in `AGENTMUX_CHANNEL`):
- `shared`, `snapshots`, `dev`, `versions` (would collide with sibling dirs at `~/.agentmux/`)
- `runtime` (used inside channels)

### 2.4 Single-instance enforcement is per-channel

The named-pipe lockfile lives at `~/.agentmux/channels/<channel>/runtime/lockfile`. Two binaries of *the same channel* are mutually exclusive (single-instance per channel — the existing launcher invariant, now scoped correctly). Different channels can run concurrently: `stable` and `local-<branch>-<hash>` and `dev-<branch>` simultaneously, each with its own srv on its own dynamic port, each in its own data dir. This is the existing "multiple instances run in parallel" guarantee from `CLAUDE.md`, preserved and clarified — the unit of mutual exclusion is the channel, not the binary.

### 2.5 Build-time channel marker for portable

`agentmux_common::is_dev_build_exe` already distinguishes the exe's provenance. Extend to also detect "built by local `task package`" vs. "built by release CI". Cleanest: compile-time env var `AGENTMUX_BUILD_CHANNEL_DEFAULT`, set to `local-<branch>-<hash>` by `task package`'s build invocation and to `stable` by the release CI script.

```toml
# Taskfile.yml — package task
env:
  AGENTMUX_BUILD_CHANNEL_DEFAULT: local-<branch>-<hash>
```

```rust
// agentmux-common/src/data_paths.rs
const BUILD_CHANNEL_DEFAULT: &str =
    option_env!("AGENTMUX_BUILD_CHANNEL_DEFAULT").unwrap_or("stable");
```

The runtime channel resolution becomes:

```rust
pub fn resolve_channel(mode: &RuntimeMode) -> String {
    // 1. Explicit env override always wins.
    if let Ok(c) = std::env::var("AGENTMUX_CHANNEL") {
        if sanitize_channel_name(&c).is_some() {
            return c;
        }
    }
    // 2. Mode-based default.
    match mode {
        RuntimeMode::Dev { branch } => format!("dev-{}", sanitize_channel_name(branch).unwrap_or("default".into())),
        RuntimeMode::Installed | RuntimeMode::Portable => BUILD_CHANNEL_DEFAULT.to_string(),
    }
}
```

### 2.6 Path API contract — new `DataPaths` fields

Add `channel: String` to `DataPaths`. `resolve` signature:

```rust
pub fn resolve(version: &str, mode: &RuntimeMode) -> Result<Self, String> {
    let root = resolve_root()?;
    let channel = resolve_channel(mode);
    let channel_dir = match mode {
        RuntimeMode::Dev { .. } => root.join("dev").join(strip_dev_prefix(&channel)),
        RuntimeMode::Installed | RuntimeMode::Portable => root.join("channels").join(&channel),
    };

    Ok(Self {
        home_dir: root,
        channel,
        instance_dir: channel_dir.clone(),    // renamed conceptually; physical path is now channel_dir
        data_dir: channel_dir.join("data"),
        config_dir: channel_dir.join("config"),
        logs_dir: channel_dir.join("logs"),
        cef_cache_dir: channel_dir.join("cef-cache"),
        agents_dir: channel_dir.join("agents"),
        instance_runtime_dir: channel_dir.join("runtime"),
        shared_dir: root.join("shared"),
        mode: mode.clone(),
    })
}
```

`version` is still passed in (used by Increment B's migration framework as `code_version`) but no longer appears in the path. Backward-compatible at the API level — every existing call site keeps working without code changes.

Env-var export adds `AGENTMUX_CHANNEL`. `AGENTMUX_VERSION` is retained for diagnostics + the migration framework, but no longer drives path resolution.

### 2.7 No migration from old `versions/<v>/` (Increment A scope)

Per the precedent set by the original 2026-05-05 spec (§3.4 "No migration"), Increment A does **not** automatically move data from `~/.agentmux/versions/<v>/` into `~/.agentmux/channels/<channel>/`. The old dir is simply not read by the new code; user starts fresh in the new channel.

Rationale: lossy migration is worse than a fresh start. The Increment C import wizard will eventually offer "import from `~/.agentmux/versions/<v>/`" as one of its sources, but until that ships, the old dir is preserved on disk for manual recovery if needed.

Document this in the user-facing changelog for the version that introduces channels.

---

## 3. Schema migration framework (Increment B)

### 3.1 Schema version in `meta.json`

Each channel's `data/` carries a `meta.json`:

```json
{
  "schema_version": 14,
  "channel_created_at": "2026-05-24T17:30:00Z",
  "channel_created_by_version": "0.38.6",
  "last_run_version": "0.39.2",
  "last_migration_at": "2026-06-15T09:18:42Z",
  "last_migration_from": 13,
  "last_migration_to": 14
}
```

Read on srv launch; consulted by the migration logic.

### 3.2 Migration discovery + ordering

Migrations live at `agentmux-srv/src/storage/migrations/` (one file per target schema version):

```
migrations/
  v0001_initial.rs
  v0002_add_db_agent_history.rs
  v0003_split_definition_from_instance.rs
  ...
  v0014_add_skills_column.rs
  mod.rs                ← registers them in version order
```

Each migration: `pub fn up(tx: &Transaction) -> Result<()>;` — pure SQL, runs inside a single transaction.

### 3.3 Launch sequence

```rust
fn ensure_channel_schema(data_dir: &Path, code_version: u32) -> Result<()> {
    let meta = read_meta(data_dir)?;
    match meta.schema_version.cmp(&code_version) {
        Ordering::Equal => Ok(()),
        Ordering::Less => {
            snapshot_data_dir(data_dir, &meta.schema_version, code_version)?;
            for v in (meta.schema_version + 1)..=code_version {
                run_migration(v, data_dir)?;
            }
            write_meta(data_dir, code_version)?;
            Ok(())
        }
        Ordering::Greater => Err(format!(
            "this AgentMux ({}) is too old to open this channel's data \
             (schema v{}, this binary speaks v{}). Upgrade AgentMux or \
             use a different channel.",
            env!("CARGO_PKG_VERSION"),
            meta.schema_version,
            code_version,
        )),
    }
}
```

The "schema too new" branch is the **safety lock**. Surface to the user via the launcher splash with actionable text ("Update AgentMux or switch channels with `AGENTMUX_CHANNEL=...`"). Hard exit; no fallback open.

### 3.4 Snapshot policy

Before any migration sequence:

```
~/.agentmux/snapshots/<channel>-pre-v<code-version>-<ISO8601>.bak/
  ├── objects.db
  ├── sagas.db
  ├── filestore.db
  └── meta.json
```

Auto-prune to the last 5 snapshots per channel. Each snapshot ~few hundred MB worst case; budget cap of ~2 GB per channel.

Rollback (manual, post-incident): `agentmux --restore-snapshot <name>` copies the snapshot back over the channel's `data/` dir. Documented but not on the happy path.

### 3.5 Reversibility

Migrations are **forward-only**. Down-migrations are a rabbit hole — they double the schema-change cost, are rarely exercised, and are an anti-pattern in most modern frameworks (Postgres, Rails post-2010, Django).

The snapshot is the rollback mechanism. If a migration is wrong, ship a fix in the next version; the snapshot lets the user revert manually if the fix can't come fast enough.

---

## 4. Import wizard (Increment C)

### 4.1 Trigger

On srv launch into a *fresh* channel (no `meta.json`, no `data/` files), check for importable sources:

- `~/.agentmux/channels/<other-channel>/` — cross-channel
- `~/.agentmux/versions/<v>/` — legacy Phase-1 dirs
- `~/.agentmux/dev/<branch>/` — dev branch dirs (less common)

If any exist with at least one agent instance, surface a one-time import prompt via the launcher splash:

> "Found agents from `<source>` (X agents, Y conversations). Import to this channel?"
>
> [Import]  [Skip — start fresh]
>
> ☑ Keep `<source>` data intact (recommended)

### 4.2 Import scope

Default: agent definitions, agent instances, identity, memory, conversation history. Not imported: logs, cef-cache, runtime.

Implementation: `COPY` via SQL `INSERT INTO ... SELECT FROM ATTACH'd db` for SQLite, plus a recursive copy for the agent workspaces under `agents/`.

Marks the destination channel's `meta.json` with `imported_from: { channel: "...", at: "...", row_counts: {...} }` so the prompt doesn't fire again.

### 4.3 Out of scope for Increment A

Increment A only ships §2. Increment B adds §3. Import wizard is its own PR. Each is independent — channels work without migrations (single channel, ratcheted schema), migrations work without import (within a single channel's lifetime), import works on top of both.

---

## 5. Delivery plan

### 5.1 Increment A — Channels infrastructure

**Scope:** §2 only. No migrations, no import.

**PRs:**
1. `feat(common): channels — replace per-version dirs with per-channel`
   - `agentmux-common/src/data_paths.rs` — new `channel` field, new `resolve_channel`, updated `resolve`, sanitize rules.
   - `agentmux-common/src/runtime_mode.rs` — no change.
   - Comprehensive unit tests for sanitization, mode→channel mapping, env-var override, path layout.
   - Plumb `AGENTMUX_CHANNEL` through env-var export.
   - Taskfile.yml: `task package` sets `AGENTMUX_BUILD_CHANNEL_DEFAULT=local-<branch>-<hash>`.
   - Single-instance lockfile path updated to channel-keyed (`runtime/lockfile`).
   - **No** migration of existing `versions/<v>/` dirs (per §2.7).
   - Ride along: this spec (`SPEC_DATA_CHANNELS_2026_05_24.md`) + per `feedback_no_doc_only_prs` directive.

**Test plan:**
- Unit tests in `data_paths.rs`: every channel-resolution edge case (env override, dev branch, build marker default, invalid names rejected).
- Integration: build `local-<branch>-<hash>` portable v0.38.6, create agent "continuity-test", build v0.38.7, launch, confirm agent present.
- Integration: build with `AGENTMUX_CHANNEL=experiment` set in env at runtime → confirm data lands at `~/.agentmux/channels/experiment/`.
- Regression: existing `task dev` workflow uses `dev-<branch>` channels under `~/.agentmux/dev/<branch>/` — same on-disk path as today, no behavior change.

**Acceptance gate:** "My Agents" list persists across a patch bump + rebuild + relaunch on the `local-<branch>-<hash>` channel.

### 5.2 Increment B — Migration framework + safety lock

**Scope:** §3 only. Requires Increment A.

**PRs:**
1. `feat(srv): meta.json + schema_version + forward migration framework`
2. `feat(srv): pre-migration snapshot + auto-prune to 5`
3. `feat(srv): safety lock — refuse to open newer-schema data`

Each PR is small and focused. The migration framework (§3.2 file structure) is the biggest piece; subsequent additions are mechanical.

**Test plan:**
- Unit: each migration runs against a synthetic DB, asserts target schema present.
- Integration: write v3 data with a v3 binary, run v4 binary → expects v4 schema + snapshot at `~/.agentmux/snapshots/`.
- Integration: write v4 data with a v4 binary, run v3 binary → expects launch refusal with clear error.

### 5.3 Increment C — Import wizard

**Scope:** §4 only. Requires Increment A; benefits from B but doesn't require it.

**PRs:**
1. `feat(srv): import wizard backend — list sources, perform copy`
2. `feat(frontend): import wizard UI in launcher splash`

**Test plan:**
- Integration: import from legacy `versions/<v>/` into a fresh `local-<branch>-<hash>` channel; verify agent count + identity + memory carry over.
- Integration: declining the prompt doesn't re-prompt on next launch.

---

## 6. Rollout

| Increment | Lands on which channels first | Rollout to release builds |
|---|---|---|
| A | `local-<branch>-<hash>` activates immediately on next `task package` build | `stable` channel ships in the next `chore: release` after merge |
| B | All channels — required before any real schema change across versions | Same release as A or shortly after |
| C | All channels — first-launch behavior only | Same as B |

The release commit that introduces channels needs a clear user-facing note in `VERSION_HISTORY.md`:

> **Data location changed.** Agents, identity, memory now live under `~/.agentmux/channels/stable/` (was: `~/.agentmux/versions/<version>/`). One-time fresh start on this version; the import wizard (next release) will offer to bring forward older data. The old `versions/` dir is preserved on disk for manual recovery.

If we're not comfortable with the fresh-start UX even for the first stable release with channels, sequence as **B → C → A** for production: ship the framework + import wizard first (no behavioral change), then flip to channels in a later release where the wizard makes the migration seamless. For dev/portable, ship A immediately regardless — local-only users opt into the fresh start by definition.

---

## 7. Open questions

(Mirrored from Discussion #1026 — answers update both places.)

1. **Sequencing for production:** Increment A first (fresh start, lossy) or B+C first (seamless migration when A lands)? Recommend A-first for `local-<branch>-<hash>`, B+C+A together for the first `stable` release that ships channels.

2. **Should `task dev` consolidate into channels too?** Currently kept at `~/.agentmux/dev/<branch>/` for clarity. Branches → channels would unify the namespace but lose the visual distinction. Recommend keep dev separate; it's a different concept (per-branch isolation for feature work).

3. **Blast radius of accidental channel mismatch:** today, worst case is "lost test agents"; under channels, same. Under §3 with a buggy migration: "potentially corrupted shared state" — mitigated by the snapshot + safety lock. The added attack surface is the migration code itself; mitigated by per-migration tests + the snapshot rollback.

4. **`AGENTMUX_CHANNEL` env-var precedence:** absolute, or only when set explicitly (not via a `.env` file inherited from a parent shell)? Recommend absolute precedence — if the user set it, they meant it. Lint: srv logs `channel resolved from env override: <name>` at startup so this is visible in `muxlog srv`.

5. **Reserved channel name `default`:** currently free. Reserve it as a synonym for `stable` to avoid confusion? Recommend yes — `AGENTMUX_CHANNEL=default` should map to `stable`.

---

## 8. Out of scope

- Per-tab data isolation (different conversations within one channel). That's the existing tab/block model; orthogonal.
- Multi-user / multi-profile within a single channel. AgentMux is single-user; not in this spec.
- Network sync (cloud-backed agents). Discussed elsewhere; channels don't depend on or block that work.
- Encrypted storage at rest. Same — orthogonal.

---

## 9. References

- [`docs/research/RESEARCH_PER_VERSION_DATA_ISOLATION_2026_05_24.md`](../research/RESEARCH_PER_VERSION_DATA_ISOLATION_2026_05_24.md) — pattern survey
- [`docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md`](./archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md) — Phase 1 (per-version)
- [`agentmux-common/src/data_paths.rs`](../../agentmux-common/src/data_paths.rs) — current implementation
- [`agentmux-common/src/runtime_mode.rs`](../../agentmux-common/src/runtime_mode.rs) — mode detection (Dev/Portable/Installed)
- Discussion [#1026](https://github.com/agentmuxai/agentmux/discussions/1026) — long-term tracking thread
