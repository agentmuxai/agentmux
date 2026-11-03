# Isolation between AgentMux installs, channels, versions and test builds: what is shared today, what the design promised, and the lines to restore

**Status:** analysis. Written 2026-11-03 against `origin/main` (0.60.13 era). The fixes it ranks are built in PRs #6320 and the ones after it; section 6 lists the one check still to run.

---

## 1. The decision, and the ranked fixes

**Decision wanted from the operator:** confirm the one rule below, so build #21512 can start.

> **The rule.** Everything under a channel (`channels/<ch>/versions/<v>/…`) is private to one install and one version. Everything under `~/.agentmux/shared/` is account-wide **on purpose**, but a file there may be *written* by only one srv *version* at a time, must carry a version a reader can check before it writes, and must be declared in one list. Test builds (`task dev`, an agent's build) never see the real `shared/`.

**Why the Store and the portable install "step on each other".** They do not share a version folder: each has its own `channels/<ch>/versions/<v>/data` and its own `srv.lock`, so the isolation *per version* holds. What they share is (a) the channel-wide folders (`channels/<ch>/config`, `agents`, `wan-identity`) when both are on the same channel, which is `stable` for every release build, and (b) the whole of `shared/`, which is where the agents, their history, the accounts and the identities live. Two srvs of *different* versions can therefore run at once and both write `shared/`, each with its own schema and SQLite. Nothing coordinated them. This is not an accident in the layout; it is the account-wide design with no cross-version writer rule.

### Ranked fixes (highest risk first)

| # | Fix | Why this rank | Status |
|---|---|---|---|
| 1 | **One writer version on `shared/`** (a registration order under `shared/locks/srv-writers/`; a later srv of another version runs without `shared/` and says so) | Two versions writing the same SQLite files is what damaged the history store on one machine and, on 2026-11-02, two installs of different versions ran together for about 10 minutes on another | PR #6320 (first fix of #21274), in review |
| 2 | **A version gate that tells the person** when `shared/` is newer than the build | The three shared SQLite stores already refuse a newer `user_version` and fall back, but silently (a log line); the person saw empty agents | in #6320 (an incident) |
| 3 | **`migrate --prepare` takes the same claim** | It runs earlier than srv, in the launcher's upgrade step, and applies Global migrations to `shared/store.db` while an older srv may be running. #6320 does not cover it | build #21512 |
| 4 | **A declared list of shared paths and a CI check** that fails when new code resolves a `shared/` path that is not on the list | The creep in section 4 came from one `resolve_*` helper per feature, each added with good reason and none checked against the design | build #21512 |
| 5 | **The same writer rule for the JSON files under `shared/`** (agent registry and definitions, reactive entries, LAN instances, browser json) | They are written by every install with no version field. Lower risk than a database (atomic rename, small), higher count | after 3 and 4 |
| 6 | **Turn ownership: one live process may drive a session's next turn** | Two srvs resuming one agent session is a different harm (the 2026-07-29 retro). `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24` is the proposed fix; not checked here whether it shipped | separate task |

What #21274 needs beyond #6320 (item 3 above): `migrate --prepare` and the launcher's upgrade step must take the writer claim, and an upgrade must wait for the old srv's file to go (it already waits 20 s in srv; the launcher stops the old srv first, so this is normally instant).

---

## 2. What the design promised

| Document | What it promised about isolation |
|---|---|
| `SPEC_DATA_CHANNELS_2026_05_24` | Per-channel data under `~/.agentmux/channels/<ch>/`. "Within a channel, every version reads and writes the same data dir; agents, identity, memory, conversations all persist across patch/minor bumps. Across channels, full isolation." A forward-only migration with a **safety lock**: a binary refuses a schema newer than it knows (`check_schema_compat`). Single instance per channel. `task dev` never honors `AGENTMUX_CHANNEL`. |
| `SPEC_VERSION_ISOLATION_2026_06_01` | Phase 1: the single-instance pipe name includes the build version, so 0.40.2 and 0.41.0 run side by side. Phase 2 (built, see `data_paths.rs`): `channels/<ch>/versions/<v>/{data,logs,cef-cache,runtime}`; `config/` and `agents/` stay channel-wide. "Channels are for settings, not version isolation." |
| `SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16` and `ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14` | **Deliberately shared:** an agent's state lives in `shared/agents/`; per-channel stores are non-authoritative caches. History is the `agent:<defId>:current` zone of `shared/agents/transcripts/filestore.db`. Invariants G1/G2. |
| `SPEC_IDENTITY_STORE_SPLIT_2026_08_17` | `shared/identity-store.db` is **always global**: links, bundles, drones, MuxBus credentials, native memory, cron jobs. Accounts are the one real exception (disposable test accounts). |
| `SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27`, then `SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03` | Isolation of accounts and credentials is **opt-in** (`AGENTMUX_ISOLATED_AUTH=1`); since 2026-10-03 auth is shared by default on every channel. |
| `RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29`, `RETRO_STALE_RESUME_SESSION_ID_ACROSS_CHANNELS_2026_07_29` | Sharing identity and session data is by design, but "turn ownership/execution is per-process and has no cross-process lock or liveness check." A `task dev` is **not** a safe sandbox for a real agent's session. |

**The shape of the promise:** per version, everything mutable and private; per channel, settings and agent definitions; account-wide, the identity of the person's agents, their history and their accounts. The one guard the design relied on for the account-wide part was the `user_version` safety lock on each database. It stops an *older* build from opening a *newer* file. It does not stop two builds that agree on the schema from writing together, and it does not exist for the JSON files.

---

## 3. What is shared today

Read from `origin/main`. "Gate" is the schema/`user_version` check at open. "Lock" is what excludes a second writer. **Note:** the three shared SQLite stores *do* have a version gate (`check_schema_compat` in `Store::open_shared`, `Store::open_identity_store` and `FileStore::open_global_transcripts`); what they lack is a cross-version *writer* rule.

### 3.1 Account-wide, under `~/.agentmux/shared/` (meant to be shared)

| Path | Written by | Gate | Lock | Meant shared? |
|---|---|---|---|---|
| `store.db` (accounts, bundles, MuxBus credentials, jobs) | every srv | `user_version` (`SHARED_STORE_SCHEMA_VERSION`) | SQLite WAL; `migrations.lock.db` for migration batches | yes |
| `identity-store.db` (agent-to-account links, memory, cron) | every srv | `user_version` (`IDENTITY_STORE_SCHEMA_VERSION`) | SQLite WAL; `migrations.lock.db` | yes, always global |
| `agents/transcripts/filestore.db` (every agent's history) | every srv | `user_version` 2, or 3 with compressed frames; SQLite 3.51.3+ lock | SQLite WAL; `readers/<pid>.json` marks for the frame codec; reclaim gate | yes |
| `agents/registry/`, `agents/definitions/` (JSON per agent) | every srv | none | per-file | yes |
| `agents/reactive/` (live message targets, one file per channel) | every srv | none | per-file; startup sweep | yes (cross-channel messaging) |
| `lan/instances/` (one file per LAN-enabled instance) | every srv | none | per-file | yes |
| `identities/<bundle>/`, `providers/<dir>/` (provider logins) | provider CLIs | none (provider's own) | none | yes since 2026-10-03 |
| `browser-bookmarks.json`, `browser-profiles.json`, `browser-start-page.json` | every srv | none | temp file and rename | yes |
| `adoption-ledger.json`, `task-media/`, `messages/`, `layouts/`, `workspaces/`, `backups/`, `cli/`, `history/`, `state/bashwrap-cwd/` | various | none (some rows sit in the stores above) | mostly none | yes |
| `locks/` (`slug-migration.lock`, `build-cache-clean.lock`, `srv-writers/` after #6320), `muxbus-locks/` | the process that needs it | n/a | OS file lock | n/a |
| `migrations.lock.db` | the migration runner | n/a | `BEGIN IMMEDIATE` for the whole batch | n/a |

### 3.2 Channel-wide (shared by every version of one channel)

| Path | Written by | Gate | Lock | Meant shared? |
|---|---|---|---|---|
| `channels/<ch>/config/` (settings) | every version of the channel | none | none | yes (settings survive upgrades) |
| `channels/<ch>/agents/` | every version | none | none | yes |
| `channels/<ch>/wan-identity/wan.db` | every version | none | SQLite WAL | yes |

Every release build is channel `stable` (`BUILD_CHANNEL_DEFAULT`), so the Store install and a portable one share this block unless one is pointed at another channel.

### 3.3 Private to one version (the isolation that holds)

`channels/<ch>/versions/<v>/data/db/{objects,sagas,filestore}.db`, `data/agents/`, `cef-cache/`, `logs/`, `runtime/{srv.lock,wave.lock,ipc-port}` and the single-instance pipe (its name includes the version). `acquire_data_dir_lock` makes it one srv per data dir.

### 3.4 Machine-wide, outside any folder

| Resource | Notes |
|---|---|
| UDP 29700 and 47891 (LAN discovery) | one socket per host; the first instance holds it, the others list it as a sibling (by design) |
| Provider config outside `~/.agentmux` (`~/.claude`, `~/.codex`, keychain / credential manager) | shared by every install and by the provider CLIs themselves |
| The Store package (WindowsApps) and any portable folder | separate binaries, same `~/.agentmux` |

### 3.5 Test builds

`task dev` always names an isolated home (`AGENTMUX_HOME_OVERRIDE`), and `refuse_unlisted_shared_root` makes srv exit when it is given a data folder outside the home with no explicit shared dir. So a `task dev` does **not** see the real `shared/` unless it is told to (`AGENTMUX_SHARED_DIR`). Gaps that remain: the work store (Kanban, Presence, jobs) is isolated only for `task dev` ("any build but a `task dev` one shares the host's"); an agent's session can still be resumed by a second process (item 6); and `AGENTMUX_SHARED_DIR` is an explicit way to point a build at the real folder.

---

## 4. The creep: where sharing grew after the design

| When | What | Deliberate? |
|---|---|---|
| 2026-06 (#1387 to #1396, #1472 to #1486) | Agent definitions, the registry, workspaces and the conversation store moved from per-channel to `shared/` | Yes, wanted: agents and history follow the person |
| 2026-08-17 (#2632) | `identity-store.db` split out as "always global", with no isolation flag | Yes, wanted |
| 2026-10-03 (#4281) | Accounts, provider folders, MuxBus sign-in and per-agent cloud credentials went from isolated-by-default to shared-by-default on every channel | Yes, wanted ("no login on upgrade"); it removed the last per-channel boundary on credentials |
| 2026-10-19 | The global transcript store got a `user_version` lock because an old SQLite (3.45.0) could corrupt a WAL database several processes write | A guard added after a corruption, and the first admission that shared files need a cross-version rule |
| 2026-10-22 to 10-25 (#5324, #5328, #5330) | Compressed frames, `readers/<pid>.json` and a process scan so a reclaim waits for other srvs | Wanted; and a second, local, cross-version rule that covers only this one file |
| through 2026-10 | `lan/instances`, `agents/reactive`, `messages`, `task-media`, `browser-*.json`, `adoption-ledger.json` added under `shared/`, each with its own helper and no version field | Mostly wanted; **no one checked each against the "versioned and locked" rule** because the rule was never written down |
| 2026-11-02 | A Store install (0.60.10) and a portable one (0.60.11) ran together on one machine for about 10 minutes, both writing the shared stores; a history store on another machine was damaged earlier by the same pattern (two srvs attached at once) | Accident. It is the consequence of the above |

**The accidents are not the shared folders; they are that "shared" was allowed to mean "written by any version at any time".** The design said "within a channel, every version shares a data dir" and relied on the schema safety lock. When the data dir became per-version (Phase 2), the account-wide part moved to `shared/` without the equivalent rule.

---

## 5. The rules and guards to restore

1. **Version gate (exists):** a database refuses a newer `user_version`. Add: the person is told (done in #6320). Not added: read-only opening, because the code paths that use these stores assume they can write; the srv falls back to the channel's own data instead.
2. **Cross-version single writer (#6320):** registration order in `shared/locks/srv-writers/<n>.json`, made whole by hard link; a later srv of another version waits 20 s, then runs without `shared/` and records an incident naming the owner. Same-version srvs share as before (one SQLite, one schema, WAL across processes is safe: it is how every window of one version has always worked). A deferred srv refuses to start when its own data needs preparing.
3. **The launcher's upgrade step takes the claim** (`migrate --prepare`), so an upgrade cannot apply Global migrations under a running older srv.
4. **Per-instance by default:** a new feature gets a path under `channels/<ch>/versions/<v>/` unless it is on the shared list. Test builds keep the isolated home.
5. **A declared list and a CI check.** One file (for example `docs/shared-paths.toml`) lists each shared path: its name, owner module, why it is shared, its gate and its lock. A script (`check-shared-paths.mjs`) greps for `resolve_global_shared_root`, `resolve_shared_*` and `.join("shared")`, and fails when a call site is in a file the list does not name. This is the check that would have caught each row of section 4 on the day it was added.
6. **Turn ownership** (separate): one live process drives a session's next turn.

---

## 6. Checks

**Run:** a read-only look at `~/.agentmux` on one machine: `shared/agents/transcripts/readers/` holds one file, srv 0.60.13 (so, as expected, only one version is registered now). `channels/stable` holds `config`, `agents`, `wan-identity` and `versions`, the channel-wide block of section 3.2.

**Proposed (one, for the build, on a Linux machine):** two srv builds of different versions on one isolated home (`AGENTMUX_HOME_OVERRIDE` in a temp folder), started 1 s apart: assert the second records the `shared_stores_in_use` incident, runs without `shared/`, and the first's `store.db` is unchanged. Then stop the first and start a third of the second's version: assert it takes the folder. Never the operator's data.

---

## 7. What I did not check

- Whether `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24` shipped (item 6).
- The Windows named pipe and `wave.lock` behaviour across two versions, beyond what `SPEC_VERSION_ISOLATION` says.
- Each JSON file under `shared/` for a version field; none was found when reading the code.
- macOS and Linux specifics of provider config outside `~/.agentmux`.
