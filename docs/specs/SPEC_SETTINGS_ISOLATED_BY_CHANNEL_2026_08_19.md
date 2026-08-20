# Spec: Make `settings.json` isolated-by-default for every non-`stable` channel

**Date:** 2026-08-19
**Status:** Implemented same day (Phase 1 + 2, this PR) — see §7 for what shipped.
**Precedent (same shape of decision, already shipped):** `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (amends `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`) — this spec follows that one's mechanism almost exactly, substituting `settings.json` for the identity store.
**Related:** `docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md` is unrelated in subject but the same investigation session surfaced this — a fresh `task package` portable build was found to have `network:lan_discovery: true` even though the code's own default is `false`, traced to `settings.json` resolving to a single file shared by every channel on the machine.

## 0. Motivation

A fresh `task package` portable build (channel `local-<branch>-<hash>-<build-id>`, per `CLAUDE.md`'s "Data isolation is per-BUILD for local builds") was observed shipping with LAN discovery (`network:lan_discovery`) already **on**. The code's own default is `false` — verified independently in the wconfig struct's serde default, `build_default_config()`'s `FullConfigType::default()`, and the frontend's `!!settingsAtom()?.[...]` read. The actual cause: `settings.json` resolves to `~/.agentmux/channels/settings.json` — a single file sitting as a sibling of every `~/.agentmux/channels/<channel>/` directory, not inside any of them — so **every channel on the machine (dev, every past and future portable build, and the real installed release) reads and writes the exact same file.** The setting wasn't defaulting to on; it was carrying forward a value the user had toggled on at some point in the past, in some other channel, indefinitely.

This is not a bug in the sense of unintended code — both consumers (§1) document the shared location as deliberate ("the modern location," "mirrors srv"). It's a **design decision** this spec proposes changing, the same way `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` changed an equally deliberate "auth is global" decision six weeks earlier, for the same underlying reason: a fresh channel silently inheriting global state defeats the purpose of channel isolation for exactly the class of setting where a surprise carry-over matters most — anything network-facing, security-adjacent, or otherwise not purely cosmetic.

## 1. Current state

### 1.1 The infrastructure to do this already exists and is already wired most of the way there

`agentmux-common/src/data_paths.rs`'s `DataPaths` struct already computes a genuinely per-channel config directory:

```rust
// config and agents stay channel-wide so settings and agent
// definitions persist across version upgrades.
let config_dir = instance_dir.join("config");
```

with the struct's own field doc: `instance_dir/config/ — settings.json, repos.json, etc.`, and the module's layout comment is explicit:

```
//   channels/<ch>/config/                 ← settings (channel-wide)
```

This is exported to every host/srv subprocess as `AGENTMUX_CONFIG_DIR` (`to_env_vars()`, `data_paths.rs:272`). So the *intended* design already present in this file's own comments is channel-scoped settings (scoped to survive version upgrades within a channel, per the comment) — not global-across-every-channel. Two independent consumers, both downstream of this correctly-scoped value, currently defeat it:

### 1.2 Consumer 1 — `agentmux-srv`'s `resolve_settings_dir()`

`agentmux-srv/src/backend/config_watcher_fs.rs:24-48`:

```rust
pub fn resolve_settings_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTMUX_SETTINGS_DIR") { ... }
    if let Ok(dir) = std::env::var("AGENTMUX_CONFIG_HOME") {
        if !dir.is_empty() {
            let path = PathBuf::from(&dir);
            if let Some(root) = path.parent().and_then(|p| p.parent()) {
                return root.to_path_buf();
            }
        }
    }
    dirs::home_dir().unwrap_or_default().join(".agentmux")
}
```

`bootstrap.rs:463` re-exports the correctly-channel-scoped `AGENTMUX_CONFIG_DIR` value under the legacy name `AGENTMUX_CONFIG_HOME` (`std::env::set_var("AGENTMUX_CONFIG_HOME", &config.config_home)`, where `config.config_home` was itself read from `AGENTMUX_CONFIG_DIR` in `config.rs:92`). That value is `channels/<ch>/config/`. Walking up two parent directories from there lands at `channels/<ch>/` → `channels/` — one level *above* the channel root, landing on the `channels/` directory itself (which is why the live file sits at `~/.agentmux/channels/settings.json`, not inside any specific channel's own directory).

The "go up two levels" logic is a holdover from an older path shape (the function's own comment: `AGENTMUX_CONFIG_HOME = .../instances/v0.31.XX — go up two levels`, describing a *version* directory, not a *channel/config* directory) that was never revisited when the `channels/<ch>/config/` layout replaced it. Whether that makes today's behavior a latent bug or an accepted-if-accidental status quo doesn't change this spec's proposal — either way, the fix is the same: use the already-channel-scoped value directly, gated by the same channel-aware default the auth precedent established, rather than continuing to walk past it.

### 1.3 Consumer 2 — `agentmux-cef`'s `read_window_transparent_setting()`

`agentmux-cef/src/app/window_settings.rs:78-112` independently computes the same shared location for reading `window:transparent` before `CefInitialize` (needed pre-init, before the srv connection exists), and documents it explicitly as intentional:

```
//   2. $AGENTMUX_CONFIG_HOME/../../settings.json (channels-root shared
//      file — the modern location, e.g. ~/.agentmux/channels/settings.json)
```

Its own comment states it "mirrors srv's `config_watcher_fs::resolve_settings_dir()` (the file the settings UI actually edits)" — i.e. this was written with full knowledge that it was reproducing srv's shared-location behavior on purpose, specifically so both processes agree on which file is authoritative. **Both consumers must change together** — updating only one would make the CEF host's pre-init `window:transparent` read (which gates a command-line flag before any UI exists) disagree with what the settings UI itself edits.

### 1.4 What's already correctly channel-scoped and out of scope

`agentmux-srv/src/server/app_api/agent_open.rs:298-301` also reads `AGENTMUX_CONFIG_HOME`, but for a different purpose — it's the `CLAUDE_CONFIG_DIR`-equivalent handed to spawned agent CLI subprocesses, used *directly* (no parent-walking), so it already resolves to the correct per-channel `config_dir`. Not touched by this spec.

## 2. Problem, restated generally

Same shape as the auth precedent's §1: a fresh, isolated channel (`task dev`, `task package`, a custom `AGENTMUX_CHANNEL=` override) already gets its own data dir, DB, cef-cache, and (per the 2026-08-06 spec) its own identity store — except settings, which silently inherits whatever the shared file currently holds, indefinitely, with no per-channel record of what's actually been *reviewed* on that instance. For purely cosmetic keys (`window:theme`, `widget:pinned`) that's a convenience. For `network:lan_discovery` — a setting that starts a real mDNS/UDP-broadcast daemon advertising this machine on the local network — it means a brand-new build can silently inherit "yes, broadcast on the LAN" from a decision made in an entirely different context, weeks or months earlier, that the person running the new build has no reason to remember making.

## 3. Solution

### 3.1 Channel-aware default, reusing the exact mechanism the auth spec already established

Add `agentmux_common::isolated_settings_enabled()`, structurally identical to `isolated_auth_enabled()`:

```rust
/// Isolated per-channel settings.json.
///
/// Resolution order:
/// 1. `AGENTMUX_ISOLATED_SETTINGS=1` / `=0` — explicit override, always wins.
/// 2. Otherwise, defaults to isolated for every channel except `"stable"`.
///    `stable` is the real release channel and keeps the old always-global
///    behavior, for the same daily-driver reason SPEC_ISOLATED_AUTH_DEFAULT_
///    BY_CHANNEL_2026_08_06.md kept it for auth.
/// 3. If `AGENTMUX_CHANNEL` isn't set yet, stays global — conservative
///    default when channel context is unknown, not a guess.
pub fn isolated_settings_enabled() -> bool {
    match std::env::var("AGENTMUX_ISOLATED_SETTINGS").ok().as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => std::env::var("AGENTMUX_CHANNEL")
            .map(|ch| ch != "stable")
            .unwrap_or(false),
    }
}
```

Placed in `agentmux-common` alongside `isolated_auth_enabled()` (same file, `data_paths.rs`) since both `agentmux-srv` and `agentmux-cef` need to call it and already depend on `agentmux-common`.

### 3.2 What each consumer does with it

**`resolve_settings_dir()`** (`config_watcher_fs.rs`):

```rust
pub fn resolve_settings_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTMUX_SETTINGS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if agentmux_common::isolated_settings_enabled() {
        if let Ok(dir) = std::env::var("AGENTMUX_CONFIG_HOME") {
            if !dir.is_empty() {
                return PathBuf::from(dir); // channels/<ch>/config/ — no parent-walk
            }
        }
    } else if let Ok(dir) = std::env::var("AGENTMUX_CONFIG_HOME") {
        if !dir.is_empty() {
            let path = PathBuf::from(&dir);
            if let Some(root) = path.parent().and_then(|p| p.parent()) {
                return root.to_path_buf(); // unchanged legacy/global behavior
            }
        }
    }
    dirs::home_dir().unwrap_or_default().join(".agentmux")
}
```

**`read_window_transparent_setting()`'s `candidates()`** (`window_settings.rs`): insert the isolated path (`AGENTMUX_CONFIG_DIR`/`AGENTMUX_CONFIG_HOME` used directly, no parent-walk) as the highest-priority candidate *when `isolated_settings_enabled()`*, ahead of the existing channels-root-shared candidate — so on an isolated channel the pre-init read agrees with where srv will look, and on `stable` (or an explicit opt-out) the existing candidate order is unchanged.

### 3.3 First-boot behavior for a freshly-isolated channel: no seeding (blank slate)

This is the one place this spec's recommendation actually differs in spirit from a naive port of the auth precedent, and it needs to be stated explicitly rather than assumed.

**Considered and rejected: copy the shared file's current contents into the new channel's settings.json on first boot.** This would preserve today's "feels the same" experience for cosmetic keys (theme, pinned widgets, voice engine choice) — but it silently defeats the entire point for the motivating case: a seeded copy would carry `network:lan_discovery: true` right along with it, and the exact problem this spec exists to fix would still reproduce on every new channel, just via a copy instead of a shared file. A "seed everything except a hand-maintained list of sensitive keys" variant was also considered and rejected for this spec — it adds an ongoing maintenance burden (every future security/network-adjacent setting has to remember to add itself to the list, and forgetting is a silent regression back to today's behavior) for a benefit (not re-picking a theme) that's real but minor compared to the risk.

**Recommendation: no seeding.** A freshly-isolated channel starts with no `settings.json` at all — `wconfig::read_config_file` already treats a missing file as "use defaults," exactly the same fallback that new `task dev` branches hit today for identity. This is the direct settings-domain analogue of the auth spec's own non-goal ("No auto-seeding / import wizard... it would undercut the entire point of this spec" — §5) and inherits the same accepted cost: re-configuring cosmetic preferences on a fresh channel is real, deliberate friction, not an oversight.

## 4. What this changes in practice

| Channel | Before this spec | After this spec |
|---|---|---|
| `stable` (real installed/portable release) | Global (`~/.agentmux/channels/settings.json`) | **Unchanged — global** |
| `dev-<branch>` (`task dev`) | Global | **Isolated by default** — fresh `channels/dev-<branch>/config/settings.json`, starts empty |
| `local-<branch>-<hash>-<build-id>` (`task package`) | Global | **Isolated by default** — the exact case that motivated this spec |
| Custom `AGENTMUX_CHANNEL=…` override | Global | **Isolated by default** |

## 5. Migration / rollout

No data migration. Pure default-computation change, same as the auth precedent — the existing shared `~/.agentmux/channels/settings.json` is untouched and remains exactly what `stable` reads. Nothing deletes or rewrites it.

Rollout risk: a developer's existing `task dev` branch or an existing `local-*` portable build, on first launch after this ships, starts reading an empty (or not-yet-existent) per-channel settings.json instead of their accumulated shared preferences — window theme, pinned widgets, any voice API key, all reset to defaults on that specific channel. This is real and immediate, not hypothetical, for anyone with in-flight `task dev`/`task package` channels at rollout time. Same class of cost the auth spec accepted explicitly in its own §0/§4 ("someone's muscle-memory `task dev` now asks them to log in again... an accepted, deliberate cost"); mitigate via a log line and changeset wording, not by softening the default.

## 6. Non-goals

- **No per-key tiering** (some settings global, some isolated). Considered in §3.3 and rejected for this spec — the whole-file isolation the auth precedent uses is simpler, has no "did we remember to list this key" failure mode, and this spec's own motivating case is exactly why a hand-maintained exception list is the wrong shape here.
- **No change to `provider_auth_dir()`, the agent registry, or global transcripts.** Unaffected — this spec is scoped to `settings.json` only.
- **No change to `stable`'s behavior, ever, under any resolution path** — same invariant the auth spec holds, for the same reason (the daily-driver instance must never regress).
- **No UI treatment** ("this channel starts with default settings") beyond a log line. A UI affordance is a candidate follow-up, not required here.

## 7. Implementation phases

### Phase 1 — Channel-aware default (1 PR)

- `agentmux-common/src/data_paths.rs`: add `isolated_settings_enabled()` per §3.1, next to `isolated_auth_enabled()`.
- `agentmux-srv/src/backend/config_watcher_fs.rs`: update `resolve_settings_dir()` per §3.2.
- `agentmux-cef/src/app/window_settings.rs`: update `read_window_transparent_setting()`'s `candidates()` per §3.2.
- Add a boot-time `tracing::info!` in srv distinguishing the four resolvable states (mirroring the auth spec's own logging addition): `global — stable channel`, `global — explicit opt-out`, `isolated — channel default`, `isolated — explicit opt-in`.
- Tests: `resolve_settings_dir` needs cases for stable (unchanged global), non-stable default (isolated, no parent-walk), explicit `AGENTMUX_ISOLATED_SETTINGS=0` opt-out on non-stable, explicit `=1` opt-in, and unset-channel fallback (stays global). Mirror the auth spec's test-splitting approach (§6 Phase 1 there) rather than editing existing assertions in place.

### Phase 2 — Docs (1 PR, no code)

- `CLAUDE.md`: note that `settings.json`, like auth, is now channel-isolated by default for any channel other than `stable`.
- Changeset: `task changeset -- minor "feat(config): non-stable channels default to isolated per-channel settings.json"` — user-facing behavior change.

## 8. Open questions

1. **Should the opt-out env var be a new `AGENTMUX_ISOLATED_SETTINGS`, or should this and `AGENTMUX_ISOLATED_AUTH` collapse into one `AGENTMUX_ISOLATED` flag governing both?** Recommend keeping them separate for now — a developer debugging something settings-unrelated shouldn't have to reason about whether flipping one flag also changes auth behavior, and the two were shipped independently. Revisit only if a third isolable concern shows up and the pattern starts feeling repetitive.
2. **Does anything read `settings.json` before `AGENTMUX_CHANNEL` is set in its own process env**, the same edge case the auth spec flagged for `DataPaths::resolve()` (open question 2 there)? `window_settings.rs::read_window_transparent_setting()` runs pre-`CefInitialize`, i.e. very early in the CEF host process — confirm it always has `AGENTMUX_CHANNEL` available by that point (it should, since the launcher sets the full `to_env_vars()` set before spawning host) before merging Phase 1.
3. **CI**: same check the auth spec ran — confirm no CI job depends on a populated shared `settings.json` for a `task dev` boot.

## 9. Verification

1. `cargo test -p agentmux-common && cargo test -p agentmux-srv && cargo test -p agentmux-cef` — new/updated tests from Phase 1 pass.
2. `task package` a fresh local build with no prior `settings.json` for its (new, unique-per-build) channel. Confirm via `muxlog srv grep "settings"` the boot log reads `isolated — channel default`, and the running instance's `network:lan_discovery` reads `false` in the HostPopover regardless of what the shared `~/.agentmux/channels/settings.json` currently holds.
3. Toggle a setting (e.g. `window:theme`) on that fresh build. Confirm it writes to the per-channel path (`~/.agentmux/channels/<that build's channel>/config/settings.json`), not the shared file — the shared file's own `window:theme` value is unchanged after the toggle.
4. Boot a real `stable`-channel build (installed or a stable-labeled portable). Confirm **zero change**: still reads/writes the same shared `~/.agentmux/channels/settings.json` as before this spec, including whatever `network:lan_discovery` value is currently set there.
5. `AGENTMUX_ISOLATED_SETTINGS=0 task dev` on a dev branch. Confirm the explicit opt-out restores the old shared-file behavior.
6. Linux/macOS specifically: confirm `window:transparent` still reads correctly pre-`CefInitialize` on both an isolated dev channel (reading its own fresh/empty file → default false) and on `stable` (reading the shared file, unchanged) — this is the one setting read before srv is even up, so it's the highest-risk regression surface for Phase 1.
