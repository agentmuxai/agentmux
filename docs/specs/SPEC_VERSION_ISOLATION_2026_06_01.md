# Version Isolation — Spec & Fix Plan

**Status:** P0 bug confirmed, fix specced  
**Date:** 2026-06-01  
**Author:** AgentA  
**Tracking:** open — no PR yet

---

## 1. The Bug

When two different release versions (e.g. 0.40.2 and 0.41.0) are run simultaneously, the second one silently activates the first's window instead of launching. The user double-clicks 0.41.0 and sees 0.40.2 come to the foreground.

**Root cause:** Both versions bake `BUILD_CHANNEL_DEFAULT = "stable"` at compile time. The data dir resolves to the same path (`~/.agentmux/channels/stable/data/`). The single-instance pipe name is `hash(data_dir)` — identical for both. The second launcher gets `ERROR_ACCESS_DENIED` on the pipe bind, treats this as "already running", and forwards an "open new window" message to the existing instance (0.40.2) then exits.

**Secondary risk:** If 0.41.0 IS the first to start, it may run schema migrations on the shared DB (`objects.db`, `sagas.db`). When 0.40.2 later opens the same dir it sees a schema version it doesn't understand.

---

## 2. Design principle (user-stated)

> "Channels should only be applicable to settings, not version isolation."

What this means concretely:

| Concern | Unit of isolation |
|---|---|
| Single-instance enforcement | **Version** — two different versions must be able to run simultaneously |
| Data dir (DB, agents, conversations) | **Version** — each release writes to its own dir |
| Settings / config profile | **Channel** — channels let a user have a "work" vs "personal" config that spans versions |
| Shared identity (auth tokens, cookies) | **Account** — `~/.agentmux/shared/` stays truly global |

The current model inverted this: channel = data isolation unit, version = irrelevant. That was intentional (see §3 below) but breaks the fundamental multi-version concurrency guarantee in CLAUDE.md.

---

## 3. Why the channel model was introduced (and what it got right)

`SPEC_DATA_CHANNELS_2026_05_24.md` §1 explains: before channels, data was keyed on `versions/<v>/`, so every patch bump wiped the user's agents. The channel model (`channels/stable/`) let agents persist across 0.40.1 → 0.40.2 → 0.41.0 because all three versions shared the same dir.

That goal (agent persistence across updates) is **correct and must be preserved** in the fix.

The bug is that the channel model used the channel as **both** the settings/profile identifier AND the runtime-isolation boundary. It should only be the former.

---

## 4. Comparison: how other apps handle this

| App | Data dir key | Single-instance key | Multi-version support |
|---|---|---|---|
| VS Code | `$APPDATA/Code/` (flavor, not version) | File lock per profile | No official multi-version; users use `VSCODE_PORTABLE` |
| Chrome | `User Data/` (global, version-independent) | `SingletonLock` file | No — second version forwards to first |
| Firefox | Per-profile path | `.parentlock` file | No official support; profiles are manual |
| macOS sandboxed apps | Per bundle-ID container | OS-enforced | Major version = new bundle ID = new sandbox |
| **AgentMux (current)** | Per channel | Named pipe keyed on `hash(data_dir)` | **Broken** — same channel = same pipe |
| **AgentMux (target)** | Per version, within channel | Named pipe keyed on `hash(data_dir + version)` | **Fixed** — each version gets its own pipe |

**Key insight from industry:** No major app actually solves "two versions sharing data simultaneously" well. The correct answer is: **don't share live data between versions**. Chrome and VS Code avoid the problem by making updates in-place (the new version IS the old slot). AgentMux needs true concurrent multi-version support, which requires proper per-version isolation.

---

## 5. Fix plan

### Phase 1 (immediate, unblocks multi-version) — P0

**Change the pipe/lock hash to include the build version.**

`agentmux-launcher/src/hash.rs`:
```rust
// Before:
pub fn data_dir_hash16(data_dir: &Path) -> String {
    let canonical = ...;
    format!("{:016x}", fnv1a_64(canonical.to_string_lossy().to_lowercase().as_bytes()))
}

// After:
pub fn data_dir_hash16(data_dir: &Path, version: &str) -> String {
    let canonical = ...;
    let combined = format!("{}\x00{}", canonical.to_string_lossy().to_lowercase(), version);
    format!("{:016x}", fnv1a_64(combined.as_bytes()))
}
```

All callers in `main.rs` (1 call site — `dir_hash` is computed once and reused for pipe, srv-pipe, and splash) and `diag.rs` (2 call sites) pass the build version string.

**Effect:** 0.40.2 and 0.41.0 now produce different pipe names → both launch independently → no single-instance collision. Data dirs still shared (same `channels/stable/data/`), but both instances won't be running simultaneously on the same DB in practice (users typically upgrade, not run side-by-side long-term).

**Risk:** Low. The pipe name is internal; no external contract. The splash event name uses the same hash — changing it means a new 0.41.0 instance won't find 0.40.2's splash to dismiss it (acceptable, they're different processes).

**Breaking change:** None externally. Old launchers can't forward to new ones on the same channel, but that was already broken (different schemas).

---

### Phase 2 (structural, prevents DB collisions) — P1, next minor

**Version-scope the data directory within the channel.**

New layout:
```
~/.agentmux/
├── shared/                          # auth, cookies — unchanged
├── channels/<channel>/
│   ├── versions/<semver>/           # NEW — per-version data
│   │   ├── data/                    # objects.db, filestore.db, sagas.db
│   │   ├── logs/
│   │   └── cef-cache/
│   ├── config/                      # channel-level settings (span versions)
│   └── runtime/                     # IPC pipes, lock — keyed on version (from Phase 1)
└── dev/<branch>/                    # dev mode — unchanged
```

`DataPaths::resolve()` changes: `data_dir` becomes `channels/<channel>/versions/<semver>/data/` for Installed/Portable modes.

**Agent persistence:** `agents/` stays at the channel level (`channels/<channel>/agents/`) — shared across all versions of the same channel. Only the runtime DB (objects, sagas, filestore) is version-scoped. This preserves the "agents survive patch bumps" goal while preventing DB schema collisions.

**Migration:** On first run of a new version, if `channels/<channel>/versions/<prev-semver>/data/` exists, offer to copy or migrate it. The migration wizard planned in `SPEC_DATA_CHANNELS_2026_05_24.md` §Increment-C applies here.

---

### Phase 3 (settings model) — future

**Decouple channel from data path entirely.**

Make channels purely a settings/config namespace:
- `channels/<channel>/config/` — settings.json, keybindings, themes (user's "work profile" vs "personal profile")
- Data is always version-scoped: `~/.agentmux/versions/<semver>/data/`
- Channel is selectable at launch (like VS Code's "portable" flag), not baked at compile time
- Default channel: derived from the binary's signing identity / distribution channel (stable, beta, nightly)

This is the principled end-state the user described. Phase 1 + 2 are the path to get there without breaking existing users.

---

## 6. What changes in Phase 1 (files + callsites)

| File | Change |
|---|---|
| `agentmux-launcher/src/hash.rs` | Add `version: &str` param to `data_dir_hash16`, include in hash input |
| `agentmux-launcher/src/main.rs` | Pass `env!("CARGO_PKG_VERSION")` to the 1 `hash::data_dir_hash16` call; `dir_hash` reused for pipe + srv-pipe + splash. Pass `AGENTMUX_IPC_HASH` env to host at both spawn sites (Windows + Unix). Update `forward_open_new_window` to read `ipc-port-{hash}`. |
| `agentmux-launcher/src/diag.rs` | Update 2 call sites; `version` already in scope |
| `agentmux-cef/src/main.rs` | Write `ipc-port-{AGENTMUX_IPC_HASH}` instead of `ipc-port` |
| `agentmux-common/src/data_paths.rs` | **(Phase 2)** Version-scope `data_dir`, `logs_dir`, `cef_cache_dir`, `instance_runtime_dir` under `channels/<ch>/versions/<v>/`. `config_dir` and `agents_dir` stay channel-wide. |
| `agentmux-launcher/src/data_dir.rs` | **(Phase 2)** `migrate_legacy_data_dir()`: on first run copies `channels/<ch>/data/db/` → `channels/<ch>/versions/<v>/data/db/` if old path exists and new doesn't |

That's the entire Phase 1 surface. No changes to `data_paths.rs`, no changes to the DB layout, no migration needed.

---

## 7. What Phase 1 does NOT fix

- **Simultaneous write conflict:** If a user somehow runs 0.40.2 and 0.41.0 on the same `channels/stable/data/` simultaneously (each having started fresh because they now have different pipes), both will try to write to the same SQLite files. SQLite handles concurrent readers fine but concurrent writers will serialize via WAL. This is an edge case (most users run one version at a time) and Phase 2 eliminates it completely.
- **Schema forward-migration hazard:** If 0.41.0 runs migrations on the shared DB, 0.40.2 started after will see a schema it doesn't understand. Phase 2 eliminates this by separating the DB dirs.

---

## 8. For future agents

- **Phase 1 is a P0 fix** — any release where two versions can coexist on the same machine is broken without it. Always include the build version in the pipe hash.
- **Do not change the channel name** (`stable`) to include the version — that would break the agent-persistence goal and is NOT the right fix. The fix is in the hash input, not the channel name.
- **The single-instance pipe contract:** `\\.\pipe\agentmux-<hash>\command` where hash = first 16 hex chars of `fnv1a_64(lowercase(canonical_data_dir) + "\x00" + build_version)` after Phase 1. The `\x00` separator ensures path and version strings can't be confused. Document this if the pipe format ever needs updating.
- **Channels are for settings.** If you're adding a feature that scopes something by "which version the user is on", it should use the version number, not the channel. Channels are the user's configuration profile, not a version identifier.
- **Discussion #1026** is the long-term tracking thread for data isolation work. Append decisions and PRs there, don't fork.
