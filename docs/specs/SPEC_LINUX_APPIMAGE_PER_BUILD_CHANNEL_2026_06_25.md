# SPEC: Linux AppImage Per-Build Channel Isolation

**Date:** 2026-06-25
**Status:** Proposed
**Author:** smike (agent)
**Area:** `scripts/package-linux.sh` (new) · `Taskfile.yml` · `scripts/build-appimage-linux.sh`

---

## 1. Problem

`task package:linux` always bakes the `stable` channel into the AppImage binary.
When a user (or developer) runs the freshly-built AppImage alongside a release
AppImage of the same version, both share the same channel key → same data dir,
same CEF-cache dir, and same single-instance pipe. The new binary joins the
existing instance instead of launching independently.

This mirrors the pre-#1315 Windows bug: running a local build would steal focus
from or collide with an existing release installation.

---

## 2. Root cause

`AGENTMUX_BUILD_CHANNEL_DEFAULT` is baked at **Rust compile time** via
`agentmux-common/build.rs`:

```rust
if let Ok(channel) = std::env::var("AGENTMUX_BUILD_CHANNEL_DEFAULT") {
    println!("cargo:rustc-env=AGENTMUX_BUILD_CHANNEL_DEFAULT={channel}");
}
```

The fallback when the env var is absent is `"stable"` (in `data_paths.rs`).

On **Windows**, `scripts/package.sh` computes the channel (`local-<branch-slug>-
<branch-hash>-<build-id>`) and exports `AGENTMUX_BUILD_CHANNEL_DEFAULT` **before**
running cargo — so the channel is stamped into every compiled crate.

On **Linux**, `task package:linux` declares `build:host`, `build:backend`, and
`build:frontend` as **Taskfile deps** — they run before the bash command, without
the env var set. By the time `build-appimage-linux.sh` executes, all binaries are
already compiled with the `stable` fallback.

macOS has the same gap (`task package:macos` also bypasses `scripts/package.sh`).

---

## 3. Design

### 3.1 Channel format

Use the **same scheme as Windows**: `local-<branch-slug>-<branch-hash>-<build-id>`.

- `branch-slug` — human-readable prefix of the git branch, coerced to `[A-Za-z0-9._-]`, capped at 27 chars.
- `branch-hash` — 6-char SHA1 of the full branch name (disambiguates long branches that share a prefix after truncation).
- `build-id` — 8-char SHA1 of the full build label (`<ver>+g<sha>[.dirty].<stamp>.<pid>`); unique per build even for concurrent same-second builds of the same branch.

Total: `"local-"(6) + slug(≤27) + "-"(1) + hash(6) + "-"(1) + build-id(8)` = ≤55 chars, well under the 64-char cap in `data_paths.rs::sanitize_channel_name`.

**Release override**: `RELEASE_CHANNEL=stable` (set by CI, never by `task package:linux`).

### 3.2 Build label

Same as Windows: `<version>+g<sha>[.dirty].<stamp>.<pid>` — semver build metadata,
never affects version precedence, names the output file so builds are unique on disk.

Output filename: `AgentMux_<label>_amd64.AppImage` for local builds,
`AgentMux_<version>_amd64.AppImage` for release builds (`RELEASE_CHANNEL=stable`).

### 3.3 Orchestrator script

Introduce `scripts/package-linux.sh` — mirrors `scripts/package.sh` for Linux:

1. Parse `[--fresh] [output-dir]` flags (--fresh kept as no-op for back-compat).
2. Compute `VERSION`, `BRANCH`, `SHA`, `DIRTY`, `STAMP`, `LABEL`, `CHANNEL`.
3. Honor `RELEASE_CHANNEL` override.
4. Export `AGENTMUX_BUILD_CHANNEL_DEFAULT="$CHANNEL"` and `AGENTMUX_BUILD_LABEL="$LABEL"`.
5. Run the full build pipeline: `task build:host build:backend build:frontend copy:schema && task bundle`.
6. Call `bash scripts/build-appimage-linux.sh "$OUTDIR"` (passing the output dir).
7. Print the final AppImage path, channel, and label.

### 3.4 Taskfile change

Replace `task package:linux` body:

```yaml
# BEFORE
package:linux:
    deps: [build:host, build:backend, build:frontend, copy:schema]
    cmds:
        - task: bundle
        - bash scripts/build-appimage-linux.sh

# AFTER
package:linux:
    cmds:
        - bash scripts/package-linux.sh {{.CLI_ARGS}}
```

The orchestrator owns the build order so `AGENTMUX_BUILD_CHANNEL_DEFAULT` is set
before any cargo invocation.

`task package:release:linux` (release CI path) remains:
```yaml
package:release:linux:
    cmds:
        - bash -c 'RELEASE_CHANNEL=stable bash scripts/package-linux.sh {{.CLI_ARGS}}'
```

---

## 4. Isolation invariants

These match the Windows guarantees (see `SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`):

- **I1** — single-instance pipe key = `hash(data_dir + version)`; `data_dir` is
  derived from the channel, so each local build gets a unique pipe. A
  freshly-built AppImage always launches as its own instance.
- **I6** — agents and auth are global (`~/.agentmux/shared/`); only pane
  layout and memories start fresh per build. Users stay logged in across builds.

---

## 5. Files changed

| File | Change |
|---|---|
| `scripts/package-linux.sh` | **New** — Linux build orchestrator (mirrors `scripts/package.sh`) |
| `Taskfile.yml` | `package:linux` → delegates to `scripts/package-linux.sh`; add `package:release:linux` |
| `scripts/build-appimage-linux.sh` | Read `AGENTMUX_BUILD_LABEL` for output filename (fallback: `AgentMux_<version>_amd64.AppImage`) |
| `CLAUDE.md` | Update `task package:linux` description to note per-build channel |

macOS gap (`task package:macos`) is out of scope here — tracked as a separate follow-up since macOS packaging involves notarization complexity.

---

## 6. Acceptance

- `task package:linux` run alongside a running release AppImage: launches as a **separate window** with its own data dir.
- `RELEASE_CHANNEL=stable bash scripts/package-linux.sh`: produces an AppImage with `stable` channel (CI path, filename = `AgentMux_<version>_amd64.AppImage`).
- `cargo check -p agentmux-common` and `cargo check -p agentmux-launcher` pass.
- `muxlog ls` shows distinct SOURCE entries for local vs release instances.
