> **⚠️ SUPERSEDED — 2026-06-13.** Retained for its design rationale and the inbound code/doc references that cite it. For the current, code-anchored architecture of agent data & cross-channel persistence, see **[ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md](../architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md)**.

# Data directory status — where AgentMux writes things, 2026-05-09

**Question:** "why are we maintaining things outside of `~/.agentmux/`? I see `ai.agentmux.cef`?"

**Short answer:** We're not — anymore. The data-dir unification (`SPEC_DATA_DIR_UNIFICATION_2026-05-05.md`, implemented in PR #695) shipped. Everything AgentMux writes today is under `~/.agentmux/`. The `ai.agentmux.cef.*` references the user is seeing are **dead paths** in test harness code that I touched yesterday (the auth-file lookup in `tools/tests/authfile.ps1`). Those references are residue from the pre-unification harness and should be removed.

---

## What actually lives where, today

### `~/.agentmux/` — the only root

```
~/.agentmux/
├── versions/<version>/                ← installed + portable per-version state
│   ├── data/                          ← srv DB (objects.db, sagas.db, …)
│   ├── config/                        ← settings.json, providers.json
│   ├── logs/                          ← agentmux-host-vX.Y.Z.log.<date>
│   ├── cef-cache/                     ← Chromium per-version cache
│   ├── agents/                        ← agent working dirs
│   └── runtime/                       ← per-instance runtime files
├── dev/<branch>/                      ← task dev per-branch state (mirrors versions/)
│   ├── data/                          ← authkey.dev lives here
│   ├── config/, logs/, cef-cache/, agents/, runtime/
├── shared/                            ← account-wide, version-independent
│   ├── chromium-cookies/, credentials/, agent-cache/
├── agents/, shell/, tool-build-cache/ ← legacy account-wide locations
└── 0.33.6XX/                          ← LEGACY per-version dirs (cli configs);
                                        pre-unification leftovers, safe to delete
```

Path resolution lives in **one place**: `agentmux-common/src/data_paths.rs::DataPaths::resolve` (`agentmux-launcher/src/data_dir.rs` is a compat shim that just calls into it). The launcher exports `AGENTMUX_DATA_DIR` / `AGENTMUX_CEF_CACHE_DIR` / etc. as env vars; host + srv read those rather than re-resolving. **There is one path-resolution truth source, not three.**

### What's NOT there anymore

- **`%APPDATA%\ai.agentmux.cef.*`** — confirmed empty on this machine; the historical Tauri-era + early-CEF-era home for installed-mode config/data. Migrated to `~/.agentmux/versions/<v>/config/`.
- **`%LOCALAPPDATA%\ai.agentmux.cef.*`** — confirmed empty. Was the Chromium cache. Now `~/.agentmux/versions/<v>/cef-cache/`.
- **`%LOCALAPPDATA%\ai.agentmux.app.*`** — Tauri-era leftovers, dead. Spec called these out for cleanup.
- **`<portable-extract>/data/`** — portable builds used to bundle data inside the extract folder, defeating the "share account-wide caches across versions" goal. Now portable instances also write to `~/.agentmux/versions/<v>/`.

---

## What I touched this session — and what's stale

In yesterday's auth-path fix (PR #766), I added `%APPDATA%\ai.agentmux.cef.*` to the search list of `Get-AgentMuxAuthFile`. That's **literally hunting an empty directory** — those paths haven't been written by AgentMux since PR #695 shipped weeks ago. The search keeps it for theoretical "stale-machine compatibility" but in practice it never fires. **Should be removed** in a follow-up cleanup.

Same goes for the README + spec / analysis files that mention `ai.agentmux.cef.*` paths; they document the pre-unification state. Most are clearly dated (e.g. `pane-tearoff-bug-status-2026-04-07.md`) so the staleness is obvious. Worth a `grep -l ai.agentmux.cef docs/ tools/` sweep to flag anything that should be updated to reference the new layout, but it's a docs-cleanup task, not a code question.

---

## Why the spec was important

`SPEC_DATA_DIR_UNIFICATION_2026-05-05.md` cataloged four problems that were real:

1. **Three independent dev-mode detections** in launcher / cef / sidecar with subtle disagreements.
2. **Empty-string `AGENTMUX_DEV` propagation** misclassified release builds as dev when terminals inherited it.
3. **Portable bundles bloated** with Chromium cache and dead state.
4. **No version key in dev mode** — every dev branch shared `ai.agentmux.cef.dev`.

PR #695 fixed all four by:
- Centralizing path resolution in `agentmux_common::DataPaths`.
- Replacing `AGENTMUX_DEV` heuristics with explicit `AGENTMUX_RUNTIME_MODE=dev:<branch>` (or `installed` / `portable`).
- Moving portable cache out of the extract folder into `~/.agentmux/versions/<v>/cef-cache/`.
- Per-branch dev paths (`~/.agentmux/dev/<branch>/`) instead of single shared `ai.agentmux.cef.dev`.

The on-disk view confirms the spec landed: `~/.agentmux/dev/agenta-perf-baseline-retro/data/authkey.dev` is the path my latest task dev wrote. No `%APPDATA%` writes happened.

---

## Recommended follow-ups (small, isolated)

1. **Remove dead `%APPDATA%\ai.agentmux.cef.*` from the auth-file lookup.** The harness should only search `~/.agentmux/dev/<branch>/data/` and `~/.agentmux/versions/<v>/data/`. ~5 LOC. Rides with the next test-harness PR.
2. **Sweep doc references.** `grep -l ai.agentmux.cef docs/` shows ~25 files; most are dated reports that are accurate-as-of-then. The current ones (`SPEC_TEST_API_ACCESS.md`, `LOG_RESOLUTION_SPEC.md`, `portable-data-dir.md`) deserve a freshness pass. Doc-only PR ride-along.
3. **Delete the legacy per-version dirs at `~/.agentmux/0.33.6XX/`**. These are cli configs from before the `versions/<v>/` migration. The `wipe-old-data-dirs.sh` script exists for this; not run on this machine. Manual one-time op.

---

## Cross-references

- `docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md` — the original plan.
- `agentmux-common/src/data_paths.rs` — the unified resolver.
- `agentmux-launcher/src/data_dir.rs` — compat shim.
- `agentmux-common/src/runtime_mode.rs` — the `AGENTMUX_RUNTIME_MODE` enum.
- `scripts/wipe-old-data-dirs.sh` — cleanup script for pre-unification leftovers.
- PR #695 — implementation.
- Memory `reference_data_dir_unification_plan.md` — pointer.
