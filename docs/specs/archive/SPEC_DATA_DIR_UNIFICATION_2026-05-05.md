> **⚠️ SUPERSEDED — 2026-06-13.** Retained for its design rationale and the inbound code/doc references that cite it. For the current, code-anchored architecture of agent data & cross-channel persistence, see **[ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md](../architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md)**.

# Data Directory Unification Plan — 2026-05-05

**Goal.** Unify the data, cache, config, and work-folder layout for portable, installed, and dev instances under a single `~/.agentmux/` root with explicit version + mode separation. Eliminate the three independent dev-mode detection paths and the silent env-var leakage that lets one instance type collide with another.

**Status.** Plan only. No code changes yet. Discovery + design done; implementation deferred to a 2-PR sequence after review.

**Scope reduction (2026-05-05, post-discussion):** No backwards compatibility / no migration code. Torch existing state; the machine is single-user and existing data is disposable. Cuts ~40% of the original plan complexity.

**Trigger.** An agent running inside portable 0.33.624 reported that `task dev` won't run, claiming "the cache folder is bound to `ai.agentmux.dev`". On-disk verification (`<portable-root>/data/cef/` populated correctly) shows the portable IS isolated, so the bug is more subtle than direct cache collision — likely env-var leakage between the portable's terminal and the dev process. Either way, the fragile conventions deserve a redesign.

---

## 1. Current state — what the code actually does

Three binaries each compute paths somewhat independently. The conventions match in installed mode but diverge in portable + dev.

### 1.1 Where paths come from

| Binary | File:line | Detection mechanism |
|---|---|---|
| **Launcher** | [`agentmux-launcher/src/data_dir.rs:64-137`](../../agentmux-launcher/src/data_dir.rs) | `runtime/` subdir → portable. `cfg!(debug_assertions)` (compile time) → dev. |
| **Host CEF cache** | [`agentmux-cef/src/main.rs:163,348-354,406`](../../agentmux-cef/src/main.rs) | `std::env::var("AGENTMUX_DEV").is_ok()` (line 163) **and** `as_deref() == Ok("1")` (line 354) — two different checks in the same file. |
| **Host fallback (no launcher)** | [`agentmux-cef/src/sidecar.rs:96-178`](../../agentmux-cef/src/sidecar.rs) | `cfg!(debug_assertions)` (compile time, line 131) — only matters when host runs without launcher (i.e. legacy `task dev`). |
| **Srv** | (passed via env from launcher) | Receives `AGENTMUX_DATA_HOME` + `AGENTMUX_DEV` from parent. Trust-based — uses what it's told. |

### 1.2 Current path layout (per mode)

| Mode | data_dir (srv DB) | config_dir | user_home_dir | CEF cache (root_cache_path) |
|---|---|---|---|---|
| **Portable** | `<root>/data/` | `<root>/data/config/` | `<root>/data/` | `<root>/data/cef/` |
| **Installed (release)** | `%LOCALAPPDATA%/ai.agentmux.cef.v0-33-639/` | `%APPDATA%/ai.agentmux.cef.v0-33-639/` | `~/.agentmux/` | same as data_dir |
| **Installed (debug)** | `%LOCALAPPDATA%/ai.agentmux.cef.dev/` | `%APPDATA%/ai.agentmux.cef.dev/` | `~/.agentmux-dev/` | same as data_dir |
| **`task dev`** (launcher present) | `~/.agentmux-dev/` (?) | `%APPDATA%/ai.agentmux.cef.dev/` | `~/.agentmux-dev/` | `%LOCALAPPDATA%/ai.agentmux.cef.dev/` |
| **`task dev`** (host standalone) | `%LOCALAPPDATA%/ai.agentmux.cef.dev/` (sidecar fallback) | same | `~/.agentmux-dev/` | same |

### 1.3 Observed on-disk reality (this machine, 2026-05-05)

```
~/.agentmux/                                      ← user_home (installed mode + portable user_home)
  0.32.10/, 0.32.100/, …, 0.33.624/, …            ← per-version subdirs already exist!
  └── cli/claude/, cli/, …                        ← per-version CLI shell config

%LOCALAPPDATA%/ai.agentmux.cef.v0-33-626/         ← installed-mode data, version-keyed
%APPDATA%/ai.agentmux.cef.v0-33-626/              ← installed-mode config, version-keyed
%APPDATA%/ai.agentmux.cef.dev/                    ← dev-mode config (no version key — global!)
%LOCALAPPDATA%/ai.agentmux.app.v0-32-43/          ← Tauri-era leftovers, dead

C:\Users\area54\Desktop\agentmux-0.33.624-x64-portable\data\
  ├── README.txt
  ├── agents/, cef/, config/, db/, logs/         ← all in one bag
  ├── ipc-port, launcher-events.log
  └── launcher-sagas.db, srv-events.log
```

Notice that:
- `~/.agentmux/<version>/cli/` already follows the desired layout (per-version) — but only for one sub-concern.
- Portable bundles **everything** including the `cef/` cache inside the portable folder, defeating the "share account-wide caches across versions" use case (cookies, dictionaries, Chromium prefs) and bloating every portable ZIP with stale cache.
- Dev mode has **no version key** — every dev run shares the same `ai.agentmux.cef.dev` dir, so two different commits of dev mode share state.

---

## 2. Problems with the current model

### 2.1 Three independent dev-mode detections (fragile)

| Site | Mechanism | Set when |
|---|---|---|
| Launcher | `cfg!(debug_assertions)` | Compile time (build profile) |
| Host CEF cache | `env::var("AGENTMUX_DEV").is_ok()` | Runtime (any value, even empty string!) |
| Host secondary | `env::var("AGENTMUX_DEV").as_deref() == Ok("1")` | Runtime (must be exactly "1") |
| Sidecar fallback | `cfg!(debug_assertions)` | Compile time |
| Sidecar→srv pass-through | `if cfg!(debug_assertions) { "1" } else { "" }` | Compile time, **passes empty string** — see 2.2 |

**Concrete bug:** [`main.rs:163`](../../agentmux-cef/src/main.rs) does `env::var("AGENTMUX_DEV").is_ok()` — `is_ok()` returns `true` even for empty-string values. Combined with sidecar's habit of setting `AGENTMUX_DEV=""` on release builds (line 228), any subprocess that inherits this empty value is **mis-classified as dev**. This is the most plausible mechanism for the user's bug: the portable's terminal pane inherits `AGENTMUX_DEV=""` and `task dev` then trips the `is_ok()` branch.

### 2.2 Empty-string `AGENTMUX_DEV` propagates

[`sidecar.rs:227-228`](../../agentmux-cef/src/sidecar.rs):
```rust
"AGENTMUX_DEV",
if cfg!(debug_assertions) { "1" } else { "" },
```

This sets `AGENTMUX_DEV=""` rather than unsetting it. Anyone using `is_ok()` (rather than `== Ok("1")`) reads this as "dev mode is on". The two checks in `main.rs` use **different** semantics — the port-file selection uses `is_ok()` (line 163), the data-dir selection uses `== Ok("1")` (line 354). Same env var, two different meanings. Easy to break by adding a third check.

### 2.3 Portable bundles everything

`<portable-root>/data/cef/` accumulates Chromium cache (cookies, BrowserMetrics, dictionaries, Crowd Deny lists, captcha providers, …). On a current portable extract this is hundreds of MB. Per the user's framing: portables should **stay portable** (lockdown filesystem) but the *user's* persistent state (cookies, login sessions, agent workspaces) should land in `~/.agentmux/`, shared across versions where it makes sense.

### 2.4 Dev mode has no version key

`ai.agentmux.cef.dev` is a single directory. Two builds of dev (e.g. one off `main`, one off `agenta/feature-x`) share state. Schema migrations from one branch can corrupt state for the other.

### 2.5 No instance-of-version isolation

Per memory, "Each instance uses its own user data directory based on version" — but the code only separates *versions*, not *instances of the same version*. Three running portables of 0.33.624 share `<portable-root>/data/`, distinguished only by being three different on-disk extract folders. There's no "instance UUID" or workspace selector.

### 2.6 `data/` folder inside portable is awkward

- Bloats the ZIP if pre-populated (it isn't, but logs/db can grow under it after launch).
- User can't trivially "factory-reset" without nuking the whole portable.
- User can't "upgrade portable" (extract new version) and keep cookies/agent workspaces.
- Version-comparison testing (running 624 alongside 639) requires re-doing every login.

---

## 3. Target design — `~/.agentmux/` unified

### 3.1 New layout

```
~/.agentmux/                                      ← single root for ALL modes
├── shared/                                       ← version-independent, account-wide
│   ├── chromium-cookies/                        ← one Chromium profile, shared
│   ├── credentials/                             ← OAuth tokens, API keys
│   ├── agent-cache/                             ← cached model responses, etc.
│   └── README.md                                ← human-discoverable
├── versions/                                     ← per-version state
│   ├── 0.33.624/                                ← installed or portable; single instance
│   │   ├── data/                                ← srv DB (objects.db, sagas.db, …)
│   │   ├── config/                              ← settings.json, repos.json, …
│   │   ├── logs/                                ← rotated daily, 7-day retention
│   │   ├── cef-cache/                           ← per-version Chromium cache (NOT cookies)
│   │   ├── agents/                              ← agent workspaces (versioned for now;
│   │   │                                          phase 2 may shift to /shared)
│   │   ├── runtime/                             ← lock + IPC files for the single instance
│   │   │   ├── ipc-port
│   │   │   ├── named-pipe
│   │   │   ├── pid
│   │   │   └── lockfile
│   │   └── instance.json                        ← {data_schema, created_at, mode}
│   ├── 0.33.639/
│   └── …
└── dev/                                          ← every dev run, branch-keyed
    ├── current/                                 ← symlink to active branch's dir
    ├── main/                                    ← per-branch isolation
    ├── agenta-feature-x/                        ← branch name slug
    │   ├── data/, config/, logs/, cef-cache/, agents/, runtime/
    │   └── instance.json {branch, commit, …}
    └── README.md
```

**Key principles:**

1. **Single root, three top-level concerns:** `shared/` for cross-version state (cookies, credentials), `versions/<v>/` for installed + portable, `dev/<branch>/` for dev.
2. **No data inside the portable directory.** Portable extracts are pure binaries — no state. State lives at `~/.agentmux/versions/<v>/` regardless of binary location.
3. **Dev = branch-keyed, not global.** Branch slug derives from `git rev-parse --abbrev-ref HEAD` at launch, falls back to `default` if not in a git repo. Schema migrations on one branch don't corrupt another.
4. **One running instance per version.** Lock + IPC files live in `versions/<v>/runtime/` — single set per version. A second launch of the same version forwards `open_new_window` to the already-running instance (Phase B.6 behavior, preserved). See §5.5.
5. **`shared/` is opt-in per data type.** Cookies are obviously shared (you don't want to log into GitHub on every version); agent workspaces are NOT shared in phase 1 (schema risk) but can move to `shared/` later.

### 3.2 Mode detection — single source of truth

Replace the three independent detections with **one** at the launcher entry point, persisted to env for downstream binaries:

```rust
// In launcher main(), runs ONCE before anything else.
fn detect_runtime_mode() -> RuntimeMode {
    // 1. AGENTMUX_RUNTIME_MODE override (testing, debugging)
    if let Ok(s) = env::var("AGENTMUX_RUNTIME_MODE") {
        return s.parse().unwrap_or(RuntimeMode::Installed);
    }
    // 2. Path-based portable detection (unchanged from today).
    if exe_dir().join("runtime").is_dir() {
        return RuntimeMode::Portable { root: exe_dir() };
    }
    // 3. Dev detection: only if the launcher binary's path is under
    //    a known dev-build dir (dist/cef-dev/, target/debug/), or
    //    if AGENTMUX_DEV_BRANCH is set (CI override).
    if exe_dir_is_dev_build() || env::var("AGENTMUX_DEV_BRANCH").is_ok() {
        let branch = git_branch_or("default");
        return RuntimeMode::Dev { branch };
    }
    RuntimeMode::Installed
}
```

Then the launcher passes `AGENTMUX_RUNTIME_MODE` (canonical, not `AGENTMUX_DEV`) to the host. Host reads it directly, no recomputation. Sidecar fallback path is removed entirely — host without launcher is no longer a supported mode (`task dev` always uses the launcher).

### 3.3 Path API contract

Single function, used by all three binaries (launcher + host + srv read it through the env):

```rust
pub struct DataPaths {
    /// `~/.agentmux/versions/<v>/` for installed/portable, `~/.agentmux/dev/<branch>/` for dev.
    pub instance_dir: PathBuf,

    /// `instance_dir/data/` — srv DB.
    pub data_dir: PathBuf,

    /// `instance_dir/config/` — settings, repo configs.
    pub config_dir: PathBuf,

    /// `instance_dir/logs/` — host + srv + launcher logs.
    pub logs_dir: PathBuf,

    /// `instance_dir/cef-cache/` — Chromium runtime cache (regenerable).
    pub cef_cache_dir: PathBuf,

    /// `instance_dir/agents/` — agent workspaces.
    pub agents_dir: PathBuf,

    /// `~/.agentmux/shared/` — cookies, credentials, cross-version cache.
    pub shared_dir: PathBuf,

    /// `instance_dir/runtime/` — lock + pipes + port files for the
    /// single running instance of this version (per §5.5: one running
    /// instance per version; second open forwards open_new_window).
    pub instance_runtime_dir: PathBuf,

    pub mode: RuntimeMode,
}
```

The launcher computes this once, calls `ensure_dirs()`, and passes the relevant paths via env to the host:

```
AGENTMUX_INSTANCE_DIR        = .../versions/0.33.639/
AGENTMUX_DATA_DIR            = .../versions/0.33.639/data/
AGENTMUX_CONFIG_DIR          = .../versions/0.33.639/config/
AGENTMUX_LOG_DIR             = .../versions/0.33.639/logs/
AGENTMUX_CEF_CACHE_DIR       = .../versions/0.33.639/cef-cache/
AGENTMUX_AGENTS_DIR          = .../versions/0.33.639/agents/
AGENTMUX_SHARED_DIR          = .../shared/
AGENTMUX_INSTANCE_RUNTIME_DIR= .../versions/0.33.639/runtime/
AGENTMUX_RUNTIME_MODE        = "installed" | "portable" | "dev:agenta-feature-x"
```

No more `AGENTMUX_DEV=""` ambiguity. No more sidecar fallback computing its own paths. Host opens files at the paths it's told, period.

### 3.4 No migration

There is no production user base. The single dev machine running this code can be wiped manually (see §6). New launcher writes to the new paths only; old paths are ignored (and can be deleted by a one-shot script).

---

## 4. Implementation plan — 2 PRs

### PR 1 — Single-shot redesign
**Scope:** introduce `RuntimeMode` + `DataPaths` in `agentmux-common`, switch all four consumers (launcher / host / sidecar / srv) at once. New layout is the only layout. No flags, no parallel old paths.

**Changes:**
- New module `agentmux-common/src/runtime_mode.rs`:
  - `RuntimeMode { Installed, Portable, Dev { branch: String } }`
  - `RuntimeMode::current() -> Self` — detects once at process start, with this priority: (1) `AGENTMUX_RUNTIME_MODE` override, (2) path-based portable detection (unchanged), (3) path-under-dev-build-dir or `AGENTMUX_DEV_BRANCH` set → `Dev`, (4) `Installed`.
- New module `agentmux-common/src/data_paths.rs`:
  - `DataPaths` struct with all eight paths from §3.3.
  - `DataPaths::resolve(version: &str, mode: &RuntimeMode) -> Self`
  - `DataPaths::ensure_dirs(&self) -> Result<()>`
- Launcher: replace `data_dir.rs` body with thin shim around `DataPaths::resolve`. Pass canonical env vars (§3.3 list) to host + srv. Delete `AGENTMUX_DEV` setting; pass `AGENTMUX_RUNTIME_MODE` instead.
- Host: replace `main.rs:163,348-354,406` paths with reads of the new env vars. Delete the dual-semantic `is_ok()` vs `Ok("1")` checks.
- Sidecar: delete `sidecar.rs` fallback (host without launcher is no longer supported). `task dev` always goes through the launcher.
- Srv: rename `AGENTMUX_DATA_HOME` reads to `AGENTMUX_DATA_DIR` (parallel for one PR; remove old name in PR 2).

**Tests:**
- Unit: `RuntimeMode::current()` for all detection cases (env override, portable path, dev path, default).
- Integration: launch launcher under each mode, assert the on-disk layout matches §3.1 exactly. Assert `~/.agentmux/versions/<v>/data/db/objects.db` exists after first run.
- Regression: smoke `task dev` from inside a portable terminal — no env-var leakage; dev process classifies as dev based on its own binary path, not inherited env.

**Estimated:** ~3-4 days.

### PR 2 — Cleanup
- Remove the `AGENTMUX_DATA_HOME` parallel name from srv.
- Delete `agentmux-cef/src/sidecar.rs::spawn_backend` and any other dead fallback code.
- Update CLAUDE.md sections referencing the old layout.
- Update README.md and BUILD.md with the new locations.
- Add a one-shot cleanup script `scripts/wipe-old-data-dirs.sh` for the dev machine (lists what would be deleted; requires `--yes` to actually rm).

**Estimated:** ~1 day.

**Total:** ~1 week instead of ~3.

---

## 5. Open questions

### 5.1 Dev branch slug stability

`git rev-parse --abbrev-ref HEAD` returns `HEAD` in detached state, branch name otherwise. Slugified for filesystem (replace `/` with `-`). What's the policy when the user changes branches mid-session? Options:
- **A.** Snapshot at launch; mid-session branch changes don't affect data path.
- **B.** Re-detect on every IPC call; data path can move while running. Painful.

**Recommendation:** A.

### 5.2 Shared dir scope — what actually goes there?

Phase 1 candidates: cookies, OAuth tokens, API keys (credentials), Chromium dictionary downloads. Out for phase 1: agent workspaces (schema risk), cef cache (regenerable).

### 5.3 What does "task dev" without a launcher do?

Today: `task dev` runs the host directly, which uses the sidecar fallback to spawn srv. Under the new design we'd remove that fallback. **Decision needed:** is the user OK with `task dev` always going through the launcher? Pro: one less code path. Con: slower iteration if launcher rebuild is slow.

**Recommendation:** Yes — the launcher rebuild is fast (small crate, few deps); the simplification is worth it.

### 5.4 The "inside the portable, run task dev" use case (the immediate bug)

The user's specific scenario: an agent inside a portable terminal runs `task dev` from the source tree. Under the new design this works cleanly: the dev launcher reads its own path (under `dist/cef-dev/`), classifies itself as `Dev { branch }`, and uses `~/.agentmux/dev/<branch>/`. The portable's `~/.agentmux/versions/0.33.624/` is untouched. **No env-var leakage problem because the new launcher does not consume `AGENTMUX_DEV` at all** — it does its own path-based detection.

The terminal's inherited `AGENTMUX_DEV=""` is harmless under the new design; we ignore it.

### 5.5 Multi-instance behavior (resolved)

**Decision:** We do NOT support multiple running instances of the same version. If a user double-clicks the same portable / runs the same install twice, the second launch's named-pipe bind hits `ERROR_ACCESS_DENIED`, and the launcher forwards an `open_new_window` request to the already-running instance via its IPC port file (preserving the Phase B.6 behavior — see `phase-b-roadmap.md`).

**Implications for the layout:**
- Only **one** `runtime/` subdir per `versions/<v>/` is needed (`pid`, `lockfile`, `ipc-port`, `named-pipe`).
- No instance-UUID complexity. No `instances/<uuid>/` top-level dir.
- Two extracts of the same version on disk still resolve to the same `~/.agentmux/versions/<v>/` and the same `runtime/` — second launch always becomes a new-window request to the first.
- Two **different** versions running concurrently is supported (different `versions/<v1>/runtime/` and `versions/<v2>/runtime/`, no contention).
- A dev run + a portable/installed run of any version is supported (different roots: `dev/<branch>/` vs `versions/<v>/`).

---

## 6. Diagnosis of the immediate bug (interim, before redesign)

**Setup:** Agent inside portable 0.33.624 portable terminal runs `task dev`, can't, claims "cache folder bound to ai.agentmux.dev".

**Verified:** `<portable-root>/data/cef/` IS populated and isolated. So the portable's *own* cache is correct; the bug is at the dev-side.

**Most likely:** Empty-string `AGENTMUX_DEV=""` leaked from sidecar.rs:228 into the portable's spawned terminal env. When `task dev` spawns the dev host, host reads `env::var("AGENTMUX_DEV").is_ok() == true` (line 163), classifies as dev, and now both portable and dev hosts try to bind to the same single-instance pipe (since dev mode shares state across runs and possibly even with another dev run).

**Workaround until PR 1 lands:**
- In the portable's terminal, before running `task dev`: `unset AGENTMUX_DEV`
- Or stronger: `env -u AGENTMUX_DEV task dev`

**Permanent fix:** PR 1 (above) replaces `is_ok()` checks with `as_deref() == Ok("1")` everywhere, and stops sidecar from setting `AGENTMUX_DEV=""` for release builds.

---

## 7. Memory pointer

Saving a memory note (`reference_data_dir_unification_plan.md`) so future sessions land on this spec when picking up data-dir work, rather than rebuilding the analysis.

---

*Plan written 2026-05-05. PR 1 is the first concrete step; awaiting user sign-off on §5 open questions before implementation begins.*
