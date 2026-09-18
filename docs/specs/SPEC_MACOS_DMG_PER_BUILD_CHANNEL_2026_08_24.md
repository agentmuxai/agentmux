# SPEC: macOS DMG Per-Build Channel Isolation

**Date:** 2026-08-24
**Status:** Implemented same day. — #3259
**Precedent (same shape of decision, same author intent, already shipped):**
`docs/specs/SPEC_LINUX_APPIMAGE_PER_BUILD_CHANNEL_2026_06_25.md` — this spec is
the macOS follow-up that doc explicitly deferred: *"macOS gap (`task
package:macos`) is out of scope here — tracked as a separate follow-up since
macOS packaging involves notarization complexity."* Mechanism is copied
almost verbatim, substituting `scripts/package-macos.sh` for
`scripts/package-linux.sh`.
**Related:** `docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` — the
per-channel `settings.json` isolation this spec now actually lets macOS use.
That spec's own investigation found `network:lan_discovery` shipping "on" in
fresh **portable** (Windows) builds; the same symptom reproduced on a local
macOS DMG build in this session, traced to this exact gap — the settings
isolation code was already correct and already shipped, but `task
package:macos` never baked a non-`stable` channel into the binary in the
first place, so the isolation had nothing to isolate.

---

## 0. Motivation

A `task package:macos` build made from `main` in an interactive session
prompted macOS's "Allow AgentMux to find devices on local networks?" dialog
on launch, despite the LAN-discovery toggle appearing off in the UI.
Investigation found the gating code itself correct — `LanDiscovery::start()`
is only ever called when `network_lan_discovery` reads `true` off the loaded
config. The actual value on disk, in `~/.agentmux/channels/settings.json`,
*was* `true` — carried over from an earlier session, in the **shared**
`stable`-channel settings file every macOS build was silently reading,
because `task package:macos` has always compiled with the `stable` channel
baked in, local dev builds included.

This is the identical failure mode `SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md`
fixed for Windows/Linux portables — but that fix only helps a channel that
isn't `stable`. macOS never produced one.

---

## 1. Root cause

`AGENTMUX_BUILD_CHANNEL_DEFAULT` is baked at **Rust compile time**
(`agentmux-common/build.rs`, `option_env!`), falling back to `"stable"` when
unset (`agentmux-common/src/data_paths.rs`).

On **Windows** (`scripts/package.sh`) and **Linux**
(`scripts/package-linux.sh`), the packaging script computes a per-build
channel and `export`s `AGENTMUX_BUILD_CHANNEL_DEFAULT` **before** invoking
the cargo builds itself — so the channel is stamped into every compiled
crate.

On **macOS**, `Taskfile.yml`'s `package:macos` task instead declares
`build:host`, `build:backend`, and `build:frontend` as Taskfile **`deps:`**,
which Task runs *before* `scripts/package-macos.sh` (the `cmds:` step) ever
gets a chance to set the env var. By the time the script runs, every binary
is already compiled against whatever `AGENTMUX_BUILD_CHANNEL_DEFAULT` was
ambient in the calling shell — normally unset, so `stable`. This is true for
**every** `task package:macos` invocation, local dev or CI release alike —
CI (`build-macos.yml`) happened to want `stable` anyway, so the bug was
invisible there; a local dev build wanted isolation and silently didn't get
it.

---

## 2. Design

### 2.1 Channel format

Identical scheme to Windows/Linux: `local-<branch-slug>-<branch-hash>-<build-id>`.

- `branch-slug` — git branch, coerced to `[A-Za-z0-9._-]`, capped at 27 chars.
- `branch-hash` — 6-char SHA1 of the full branch name.
- `build-id` — 8-char SHA1 of the full build label (`<ver>+g<sha>[.dirty].<stamp>.<pid>`).

Total ≤ 55 chars, under the 64-char cap in `data_paths.rs::sanitize_channel_name`.

**Release override:** `RELEASE_CHANNEL=stable`, set by `task
package:release:macos` and by `build-macos.yml` CI directly — never by a
plain local `task package:macos`.

**macOS-specific tool note:** macOS ships `shasum` (Perl `Digest::SHA`
wrapper), not GNU coreutils' `sha1sum` used by the Windows/Linux scripts.
`scripts/package-macos.sh` uses `shasum -a 1` for the same hashes — same
algorithm, portable to a stock macOS toolchain with no Homebrew coreutils
dependency.

### 2.2 Build label

Same as Windows/Linux: `<version>+g<sha>[.dirty].<stamp>.<pid>`, semver
build metadata, exported as `AGENTMUX_BUILD_LABEL` for local builds only
(release builds keep `CARGO_PKG_VERSION` as the single-instance pipe key, so
same-version `stable` installs continue to share it, as designed).

**Deliberate deviation from Windows/Linux — DMG filename stays version-only.**
Windows/Linux name the local artifact after the full label
(`AgentMux_<label>_amd64.AppImage`) so concurrent local builds never collide
on disk. macOS keeps `AgentMux_<version>_arm64.dmg` for both local and
release builds — unlike a Windows ZIP or Linux AppImage, a macOS DMG is a
single file a human drags into `/Applications`; renaming it away from the
familiar `AgentMux_<version>_arm64.dmg` would break the existing
Desktop-cleanup workflow (keep-latest-by-version) for no isolation benefit —
the **channel**, not the filename, is what makes each build a separate
running instance. Two local builds of the same version now simply overwrite
each other's DMG on disk, same as before this change; they still never
collide as *running instances*, which is the actual bug this spec fixes.

### 2.3 Orchestrator script

`scripts/package-macos.sh` becomes self-contained, mirroring
`scripts/package-linux.sh`:

1. Parse `[--fresh] [output-dir]` (`--fresh` kept as an accepted no-op, matching Windows/Linux — every local build is already its own isolated dir).
2. Compute `VERSION`, `BRANCH`, `SHA`, `DIRTY`, `STAMP`, `LABEL`, `CHANNEL`.
3. Honor `RELEASE_CHANNEL` override (mutually exclusive with `--fresh`).
4. `export AGENTMUX_BUILD_CHANNEL_DEFAULT="$CHANNEL"` and, for local builds only, `AGENTMUX_BUILD_LABEL="$LABEL"`.
5. Run the full build pipeline itself: `task build:frontend`, `task build:backend`, `task build:host`, `task copy:schema`, `task bundle` — in that order, before any of the existing packaging/signing/notarization logic (unchanged).
6. Everything from "Assembling AgentMux.app" onward is unchanged — bundling, CEF patch gate, signing, DMG creation, notarization all operate exactly as before, just now against binaries compiled with the correct channel.

### 2.4 Taskfile change

```yaml
# BEFORE
package:macos:
    deps: [build:host, build:backend, build:frontend, copy:schema]
    cmds:
        - task: bundle
        - bash scripts/package-macos.sh {{.CLI_ARGS}}

# AFTER
package:macos:
    cmds:
        - bash scripts/package-macos.sh {{.CLI_ARGS}}

# NEW
package:release:macos:
    cmds:
        - bash scripts/guard-stable-build.sh
        - bash -c 'RELEASE_CHANNEL=stable bash scripts/package-macos.sh {{.CLI_ARGS}}'
```

`package:release:macos` mirrors `package:release:linux`'s guard (clean tree,
HEAD is a merged `chore: release v*` commit, `AGENTMUX_STABLE_OVERRIDE=1`
escape hatch) — for a human running it locally. Unlike Linux/Windows CI,
which call their orchestrator scripts directly with `RELEASE_CHANNEL=stable`
(bypassing the Task guard, since CI already checked out a specific trusted
ref), `build-macos.yml` is updated the same way — see §2.5.

### 2.5 CI change

`build-macos.yml`'s release step called `task package:macos -- "$OUT"` with
no channel override, relying on the (previously unconditional) `stable`
fallback. After §2.4, `task package:macos` defaults to a **local** per-build
channel — so CI must now set `RELEASE_CHANNEL=stable` explicitly, matching
how `build-linux.yml` already calls its script directly:

```yaml
# BEFORE
run: task package:macos -- "$OUT"

# AFTER
run: RELEASE_CHANNEL=stable bash scripts/package-macos.sh "$OUT"
```

This is the one change in this spec that is load-bearing for the *real*
release pipeline, not just local dev ergonomics — without it, every future
GitHub release DMG would silently ship on an isolated `local-*` channel
instead of `stable`, and every user's real data dir would stop being
readable to a fresh install.

---

## 3. Isolation invariants

Match the Windows/Linux guarantees (`SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`, I1/I6):

- **I1** — single-instance behavior: a freshly-built local DMG's `.app` runs as its own instance, not joining a running `stable` install or a sibling local build's data dir.
- **I6** — agents and auth are global; only pane layout and memories (`db_bundles`) start fresh per local channel, same as Windows/Linux. `settings.json` also now correctly isolates per `SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` — this is the concrete fix for the LAN-discovery leak that motivated this spec.

---

## 4. Files changed

| File | Change |
|---|---|
| `scripts/package-macos.sh` | Self-contained orchestrator — computes/exports the channel and runs the full build pipeline itself, mirroring `scripts/package-linux.sh` |
| `Taskfile.yml` | `package:macos` → delegates to the script with no `deps:`; new `package:release:macos` task (guarded, bakes `stable`) |
| `.github/workflows/build-macos.yml` | Release step now sets `RELEASE_CHANNEL=stable` explicitly (previously relied on the now-removed unconditional default) |
| `CLAUDE.md` | New `task package:macos` / `task package:release:macos` rows in the Commands table; macOS folded into the existing per-build-channel + settings-isolation paragraph; explicit note for agents building local macOS DMGs |

---

## 5. Acceptance

- `task package:macos` run twice in a row: both DMGs are signed + notarized identically to before; each run's `.app`, when launched, uses a distinct `local-*` channel (verify via the printed `channel :` line and `~/.agentmux/channels/local-*/settings.json` existing fresh, defaulted `network:lan_discovery` = `false`, independent of whatever `~/.agentmux/channels/settings.json` (the `stable` file) currently holds).
- `RELEASE_CHANNEL=stable bash scripts/package-macos.sh`: produces a DMG whose `.app` reads/writes the shared `stable` channel, exactly as every release build has always done.
- `task package:release:macos` on a dirty tree or non-release commit: refused by `guard-stable-build.sh`, same as `package:release:linux`.
- `build-macos.yml`'s uploaded release DMG still opens the user's real (`stable`) data dir — verified by the explicit `RELEASE_CHANNEL=stable` in the workflow, not an implicit default.
