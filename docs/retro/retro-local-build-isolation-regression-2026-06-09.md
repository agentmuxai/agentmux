# Retro: Local portable builds re-coupled after version-isolation work (2026-06-09)

**Author:** AgentA
**Triggered by:** User observation — `task package` build of main (0.43.1+g...) opened a second window in the running 0.43.2 instance instead of starting isolated.

---

## 1. What happened

User asked to run a new local portable build to smoke-test the My Agents fix (PR #1312). `task package` produced `agentmux-0.43.1+gfc98732b.20260609T113945.589-x64-portable`. Launching it opened a new window inside the existing 0.43.2 running instance instead of spawning a new, independent process.

**Expected:** new build starts fresh window, user tests new binary.
**Actual:** existing instance absorbed the launch; new binary never ran.

---

## 2. Root cause

Three separate PRs solved overlapping isolation problems. The solution space had a gap that none of them closed.

### 2.1 The PRs

| PR | Problem solved | Mechanism |
|---|---|---|
| **#1141** (label-not-bump) | Cross-branch version collision: two branches bumping to the same semver share a data dir and a Desktop folder. | `task package` stops bumping. Version = semver from `Cargo.toml` (unchanged). Build label = `<ver>+g<sha>.<stamp>` (unique per build, semver build metadata). Data dir keyed on `channel + semver`, NOT build label. Spec §3.2 explicitly says "data dir stable across rebuilds of same work." |
| **#1227** (version in pipe hash) | Two release binaries at different versions (e.g. 0.40.2 + 0.41.0) sharing the same channel shared a pipe name. Second binary activated first's window. | `data_dir_hash16(data_dir, version)` — adds `CARGO_PKG_VERSION` (semver) to the pipe hash so different releases on the same channel get distinct pipes. |
| **#1141's data_paths Phase 2** (c3883356) | Two concurrent releases sharing the same channel shared a live SQLite DB — concurrent-writer hazard. | `Portable/Installed` builds now use `channels/<ch>/versions/<v>/data/` instead of `channels/<ch>/data/`. Version = `CARGO_PKG_VERSION`. |

### 2.2 Where the gap is

All three PRs use `CARGO_PKG_VERSION` (the semver core, e.g. `"0.43.1"`) as the isolation key — for the pipe hash, for the data dir path, and for the desktop label base. The build metadata stamp (`+gfc98732b.20260609T113945.589`) is **only in the folder/ZIP name** — it never reaches the pipe hash or the data dir.

So for any two local builds of the same branch between releases:

```
Build A (earlier):  CARGO_PKG_VERSION = "0.43.1"
Build B (later):    CARGO_PKG_VERSION = "0.43.1"   ← same semver, no bump happened

channel:     local-main-b28b7a      (same — baked from branch)
data_dir:    channels/local-main-b28b7a/versions/0.43.1/data/   (same)
pipe key:    hash("...versions/0.43.1/data/\x000.43.1")                 (same)
```

Build B's launcher connects to Build A's pipe, sees "already running", and forwards "open new window" to Build A then exits. **Build B's binary never runs.** The user is interacting with the old binary in a new window.

### 2.3 Why this wasn't caught by the existing tests

`different_versions_same_dir_produce_different_hashes` (added in #1227) tests that `0.43.1 ≠ 0.43.2`. It doesn't test the `0.43.1 vs 0.43.1` case, because the invariant was stated as "different release versions must be isolated" — not "different builds of the same unreleased version must be isolated."

The intent of #1141's spec §3.2 was actually to SHARE session across rebuilds:

> "Because the data dir keys on branch, rebuilding the same patch 10 times reuses one data dir — your smoke session persists across iterations instead of resetting every build."

That goal is correct. But the implementation conflated "share data" with "share the single-instance domain." The spec intended that relaunching after closing should restore the session; instead it also means a second launch while the first is running joins the first's process.

---

## 3. The tension

Two goals that need to be separated:

| Goal | Key |
|---|---|
| **Session persistence** — agents, auth, panes survive a rebuild | Stable across builds of same branch. Key = `channel + semver`. Data dir stays the same. |
| **Single-instance domain** — each distinct binary should be its own instance | Unique per build. Key = `channel + full build label (incl. stamp)`. Pipe hash should change per build. |

Currently both use the same key (`channel + semver`). They should use different keys.

---

## 4. Terminology: "dev-portable" was wrong from the start

The channel for local builds was named `dev-portable-<branch>` in PR #1027 (2026-05-24). That name conflated two orthogonal concepts:

| Term | Actual meaning |
|---|---|
| **dev** | `task dev` mode — hot reload, Vite, no launcher, data at `~/.agentmux/dev/<branch>/`. A runtime execution mode. |
| **portable** | A ZIP distribution format. Released portables use the `stable` channel. |

A locally-built portable from `task package` is neither — it's a fully compiled production binary you built on your machine. Calling it `dev-portable` implied it belonged to the same family as `task dev` (it doesn't), and that "portable" was a meaningful qualifier in the channel name (it isn't — the channel name should describe the data isolation scope, not the build format).

The consequence: every doc, comment, and diagnostic that surfaced `local-main-b28b7a` reinforced the confusion. Users reading the channel name couldn't tell whether they were looking at a dev instance or a portable instance.

### The rename

**`dev-portable-<branch>-<hash>` → `local-<branch>-<hash>`**

"local" accurately describes what this channel is: a locally-built binary, as opposed to a CI-built release. It says nothing about how it was built (Vite or Cargo, portable or installed) — which is correct, because the channel scope is the data, not the binary format.

`local-main-b28b7a`, `local-agenta-my-agents-fix-b28b7a` — unambiguous.

This rename also reclaims 7 characters of the 64-char channel-name cap, increasing the usable branch-slug length from 20 to 27.

## 5. The fix (both issues, shipped together)

### 5a. Channel rename

`package.sh`: `CHANNEL="local-${BRANCH_SLUG}-${BRANCH_HASH}"` (was `dev-portable-*`).
`data_paths.rs`, `hash.rs`, `Taskfile.yml`, `build.rs` comments updated to match.

### 5b. Pipe key uses build label for local builds

`package.sh` already exports `AGENTMUX_BUILD_LABEL` (the full label including stamp). The launcher now uses it for the pipe hash when present:

```rust
// For local builds, AGENTMUX_BUILD_LABEL is injected by package.sh and includes
// the per-build stamp. For released builds it is unset; fall back to semver.
let pipe_version = option_env!("AGENTMUX_BUILD_LABEL")
    .unwrap_or(env!("CARGO_PKG_VERSION"));
let dir_hash = hash::data_dir_hash16(&paths.data_dir, pipe_version);
```

`agentmux-launcher/build.rs` gains `cargo:rerun-if-env-changed=AGENTMUX_BUILD_LABEL` so the launcher recompiles when the stamp changes (only launcher, not the foundational common crate — see `agentmux-common/build.rs` comment for why).

Result:

```
Build A (stamp 20260609T1100):
  data_dir:  channels/local-main-b28b7a/versions/0.43.1/data/  ← shared
  pipe key:  hash("...0.43.1/data/\x000.43.1+gfc98732b.20260609T1100") ← unique

Build B (stamp 20260609T1145):
  data_dir:  channels/local-main-b28b7a/versions/0.43.1/data/  ← shared ✓
  pipe key:  hash("...0.43.1/data/\x000.43.1+gfc98732b.20260609T1145") ← unique ✓

Released 0.43.1 (no stamp):
  pipe_version = "0.43.1"   ← unchanged behavior ✓
```

Build B starts its own window using Build A's session data. Both goals satisfied.

New test `successive_local_builds_produce_different_hashes` in `hash.rs` asserts the invariant directly.

---

## 6. Timeline

| Date | Event |
|---|---|
| 2026-05-28 | PR #1141 ships: `task package` stops bumping. Build label created but only used for folder/ZIP name. Data dir + pipe both keyed on semver. |
| 2026-06-01 | PR #1227 ships: version added to pipe hash. Solves release-version collision. Still uses semver — local builds unaffected. |
| 2026-06-01 | c3883356 ships: version-scoped data dirs. Same semver key — local build isolation gap still present. |
| 2026-06-09 | User smoke-tests My Agents fix. Build at 0.43.1 joins previous 0.43.1 instance's pipe. Regression surfaced. |

---

## 6. What to do

1. **PR:** Inject `AGENTMUX_BUILD_LABEL` into the pipe hash for local builds (see §4). Small change; touches `package.sh`, `agentmux-common/build.rs`, and `agentmux-launcher/src/main.rs`. Add a test: `same_version_different_stamps_produce_different_hashes`.
2. **Workaround (now):** Close all running AgentMux instances before launching a new local portable. The new build starts cleanly; your session data is in the same data dir, so agents, auth, and panes all come back.
3. **Update CLAUDE.md** local build versioning section to clarify: data dir is shared across rebuilds (same semver), but each build's single-instance domain is unique (via build label in pipe hash, once the fix ships).

---

## 7. Lessons

- **Spec §3.2 was under-specified.** "Data dir stable across rebuilds" was the right goal but didn't separately state the single-instance domain requirement. The two concerns were collapsed into one key.
- **`CARGO_PKG_VERSION` is the wrong isolation primitive for local builds.** It's stable across all builds between releases — which is useful for data sharing but wrong for single-instance enforcement.
- **The build label was created for traceability but not wired into the isolation machinery.** The stamp is visible in the folder name on Desktop but invisible to the pipe hash. A label that doesn't reach the isolation boundary is only half-used.
- **Invariant tests covered release-version isolation, not local-build isolation.** The case "same semver, different stamps" was never asserted. Add it.
