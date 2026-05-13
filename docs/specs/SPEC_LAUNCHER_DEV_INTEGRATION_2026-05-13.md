# SPEC: Integrating `agentmux-launcher` into `task dev`

**Status:** Draft
**Author:** AgentX
**Date:** 2026-05-13
**Related specs:**
- `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` (Phase B launcher ownership model)
- `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` (LSD-1..4 durable saga log)
- `specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` (gap #8: stale state)

---

## 1. Problem

`task dev` builds *both* `agentmux-cef` and `agentmux-launcher` but only launches `agentmux-cef.exe` directly — the launcher is built then ignored.

The launcher used to be a tiny DLL-path wrapper (per its own header comment at `agentmux-launcher/src/main.rs:1-11`). Phase B grew it into the privileged process-tree owner: Job Object J0, named-pipe IPC, single-instance enforcement, srv spawn, saga coordinator, durable saga log, event log. Anything `task dev` does is missing all of those.

This spec documents *why* `task dev` skips the launcher and proposes how to put it back in the loop.

---

## 2. Current state

### 2.1 Production layout (`scripts/package-portable.sh`)

```
agentmux-{version}-x64-portable/
├── agentmux.exe              ← launcher (renamed from agentmux-launcher.exe)
├── agentmux-portable.marker
├── data/
└── runtime/
    ├── agentmux-{version}.exe          ← host (renamed from agentmux-cef.exe)
    ├── agentmux-srv-{version}-windows.x64.exe
    ├── libcef.dll
    ├── chrome_elf.dll
    ├── libEGL.dll / libGLESv2.dll / d3dcompiler_47.dll
    ├── icudtl.dat
    ├── v8_context_snapshot.bin
    ├── *.pak  (chrome_100/200_percent, resources)
    ├── locales/en-US.pak
    ├── frontend/
    └── tools/bin/
```

### 2.2 Dev layout (`Taskfile.yml` `dev:serve`)

```
dist/cef-dev/                  ← copy of dist/cef/
├── agentmux-cef.exe           ← invoked DIRECTLY at line 529
├── agentmux-launcher.exe      ← built but unused
├── libcef.dll, *.dll, *.dat, *.pak
└── locales/en-US.pak
```

The Taskfile literally does:
```sh
cd "$DEV_DIR" && LD_LIBRARY_PATH=. AGENTMUX_DEV=1 \
  ./agentmux-cef.exe --url=http://localhost:5173
```

### 2.3 Why the launcher cannot run in the dev layout today

The launcher (`agentmux-launcher/src/main.rs`) is hard-coded for the production layout:

| File:Line | Behavior |
|---|---|
| `main.rs:40` | `let runtime_dir = exe_dir.join("runtime");` |
| `main.rs:52-63` | `SetDllDirectoryW(runtime_dir)` |
| `main.rs:87-99` | If `runtime_dir` doesn't exist → log FATAL, `eprintln!("AgentMux runtime not found in: {} ...")`, `exit(1)` |
| `main.rs:902-934` | `find_cef_binary(runtime_dir)` — searches `runtime/agentmux-{version}.exe`, `runtime/agentmux-cef-{version}.exe`, then `runtime/agentmux-cef.exe` |

If you simply replaced line 529 of `Taskfile.yml` with `./agentmux-launcher.exe --url=...` today, the launcher would set DLL path to a nonexistent `dist/cef-dev/runtime/`, then fatal-exit because that directory doesn't exist.

### 2.4 How the host self-supports without the launcher

The host (`agentmux-cef/src/main.rs`) has explicit fallback paths for "`task dev` mode where the launcher is not in the loop":

| File:Line | Behavior |
|---|---|
| `main.rs:82-99` | Host re-runs `SetDllDirectoryW`, falling back to its own dir if `runtime/` is missing |
| `main.rs:176-179` | `DataPaths::from_env().or_else(|| RuntimeMode::current + DataPaths::resolve)` — env vars optional |
| `main.rs:262` | `connect_to_launcher` — non-fatal if pipe absent |
| `main.rs:271` | `connect_to_srv` — non-fatal; "task dev mode doesn't run the launcher and so doesn't set AGENTMUX_SRV_PIPE_PATH" |
| `main.rs:303-305` | If `AGENTMUX_BACKEND_WS` absent → `sidecar::spawn_backend()` — host spawns srv itself |
| `main.rs:336-342` | `AGENTMUX_DEV=1` → writes `authkey.dev` for external test harnesses |

So the host is **designed to run standalone in dev** and the no-launcher path is a first-class supported configuration. This is why `task dev` works at all.

---

## 3. What dev mode loses by skipping the launcher

Listed here so the cost is explicit when deciding whether integration is worth the engineering:

| Lost feature | Source | Impact in dev |
|---|---|---|
| Job Object J0 (`KILL_ON_JOB_CLOSE`) | `launcher/main.rs:208-220, 543-555` | `agentmux-srv` + CEF renderers can orphan if host is killed/crashes — leaves zombie processes the developer has to `tasklist | grep agentmux` and kill by PID |
| Named-pipe single-instance bind | `launcher/main.rs:243-313` | Two `task dev` runs against the same data dir would race on CEF cache lock; relevant if a dev double-clicks "Run Dev Task" |
| `open_new_window` forwarding | `launcher/main.rs:262, 809-862` | Multi-window UX can't be exercised in dev — second invocation just opens a fresh instance |
| Launcher → host CPD pipe (`AGENTMUX_LAUNCHER_PIPE`) | `launcher/main.rs:515` → `cef/main.rs:262` | Host runs "pre-Phase-B path (no IPC connection; standalone state)" — saga coordinator's `IssueCmd::Host` actions never reach a real host in dev |
| Saga coordinator + `launcher-sagas.db` | `launcher/main.rs:343-436` | LSD-1..4 (saga durability, recovery, vacuum) untestable via `task dev` |
| Event log `launcher-events.log` | `launcher/main.rs:319-328` | No crash forensics for dev sessions |
| Coordinated srv spawn (`CREATE_SUSPENDED` → J0 → resume) | `launcher/main.rs:462-578` | Dev srv lifecycle differs from production — host's `spawn_backend` path is exercised instead, masking regressions in the launcher path |
| Versioned binary discovery | `launcher/main.rs:902-934` | N/A in dev (unversioned name is the fallback) |

**The single most important loss** is that the launcher↔host IPC and saga coordinator are entirely unexercised in `task dev`. The two heaviest investments in the codebase right now (Phase B state-machine + LSD durable saga log) are tested only in package builds. Every dev iteration risks shipping a regression in either path.

---

## 4. Why `task dev` skipped the launcher historically

Not malice — incremental drift:

1. **Pre-Phase-B**, the launcher was 100 lines of `SetDllDirectoryW` + `CreateProcess`. The Taskfile's `cd $DEV_DIR && ./agentmux-cef` achieves the *same* effect on Windows (DLL search includes CWD) without the indirection.
2. **Phase B+** (specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md) added Job Object, srv spawn, IPC, sagas. The Taskfile was never updated.
3. The launcher's `runtime/` hard-coding meant that even if someone tried to add the launcher to `task dev`, it would fatal-exit on the dev layout — there's no escape hatch.
4. The host code path explicitly accommodates the no-launcher configuration ("symmetric with sidecar.rs::spawn_backend's fallback"), reinforcing the split.

The result: dev exercises the host's fallback paths; package builds exercise the launcher path. They are two different products in the same repo.

---

## 5. Proposed integration

### 5.1 Goals

1. **Default `task dev` invokes the launcher**, exercising J0, IPC, saga log, srv spawn — the same code paths production users hit.
2. **No regression in dev iteration speed** — startup latency must stay within ~500 ms of current.
3. **Minimal launcher code changes** — the launcher is privileged production code; adding a dev-only branch is preferable to restructuring.
4. **Escape hatch preserved** — `task dev:standalone` (or env var) keeps the current host-only behavior for debugging the no-launcher fallback itself.

### 5.2 Recommended approach: **Reshape `dist/cef-dev/` to the production layout**

Change the Taskfile bundle step to mirror `package-portable.sh`'s output structure, then invoke the launcher.

**No launcher code changes required** — the launcher's existing `find_cef_binary` fallback at `agentmux-launcher/src/main.rs:933` (`runtime_dir.join(format!("agentmux-cef{}", ext))`) already handles the unversioned dev binary name.

**Resulting layout:**

```
dist/cef-dev/
├── agentmux-launcher.exe   ← invoked by Taskfile
└── runtime/
    ├── agentmux-cef.exe    ← launcher's fallback at main.rs:933 finds this
    ├── libcef.dll
    ├── *.dll, *.dat, *.pak
    └── locales/en-US.pak
```

**Taskfile.yml `dev:serve` change** (around line 487-529):

```diff
-                # Copy staging dir to session dir so dist/cef/ is never locked
-                # by the running process. Next build can overwrite dist/cef/ freely.
                 DEV_DIR="dist/cef-dev"
                 rm -rf "$DEV_DIR" 2>/dev/null || true
-                mkdir -p "$DEV_DIR"
-                cp -r dist/cef/* "$DEV_DIR"/
-                echo "Copied dist/cef → $DEV_DIR (isolation)"
+                mkdir -p "$DEV_DIR/runtime"
+                # Launcher at root; host + DLLs in runtime/ (matches package-portable layout)
+                cp target/release/agentmux-launcher.exe "$DEV_DIR/agentmux-launcher.exe"
+                cp -r dist/cef/* "$DEV_DIR/runtime/"
+                # Also need agentmux-srv in runtime/ so launcher spawn_srv resolves it
+                cp dist/bin/agentmux-srv-*-windows.x64.exe "$DEV_DIR/runtime/" 2>/dev/null || true
+                echo "Built $DEV_DIR with production layout"
                 ...
-                cd "$DEV_DIR" && LD_LIBRARY_PATH=. AGENTMUX_DEV=1 ./agentmux-cef{{exeExt}} --url=http://localhost:5173
+                cd "$DEV_DIR" && AGENTMUX_DEV=1 ./agentmux-launcher{{exeExt}} --url=http://localhost:5173
```

**Linux/macOS notes:**
- `bundle:linux` already places binaries directly in `dist/cef/`. Same restructure applies — `dist/cef-dev/runtime/`.
- The launcher's Unix path (`main.rs:151-162`) uses `exec` into the host. It currently has no `runtime/` indirection on Unix because the launcher comment says "Phase B.1 is Windows-only." For Unix, we either:
  - (a) extend the launcher's Unix branch to mirror the Windows runtime/ resolution, or
  - (b) keep Unix dev on the current direct-spawn path until Phase 7 (cross-platform parity, per launcher main.rs:153).

  This spec recommends **(b)** — the Phase 7 cross-platform work is the right time to integrate Unix dev, not a workaround now.

- `install-linux-desktop.sh` currently writes `Exec=$DEV_DIR/agentmux-cef` (line 500 of Taskfile.yml). It must be updated to point at the launcher: `Exec=$DEV_DIR/agentmux-launcher` (and the launcher then resolves `runtime/agentmux-cef`).

### 5.3 Alternative: Teach the launcher about a flat layout

**Not recommended**, but documented for completeness.

Modify `agentmux-launcher/src/main.rs`:

```rust
let runtime_dir = {
    let candidate = exe_dir.join("runtime");
    if candidate.exists() {
        candidate
    } else {
        // Dev layout: launcher + host + DLLs are all flat in exe_dir.
        // Recognized when there's no runtime/ subdir but libcef.dll is
        // a sibling. Same SetDllDirectoryW target, same find_cef_binary
        // search root.
        exe_dir.to_path_buf()
    }
};
```

**Why not:**
- Adds a code path that only runs in `task dev`. The launcher is privileged production code — every conditional is a potential surprise for installed users who happen to have a missing `runtime/` (corrupt extract).
- Couples launcher behavior to layout heuristics that drift independently of `package-portable.sh`.
- A flag-gated version (`AGENTMUX_DEV_FLAT=1`) adds an env var that has to stay in sync between Taskfile and launcher.

Option 5.2 ships ~10 lines of YAML diff; this option ships Rust changes to the privileged launcher *and* a Taskfile change. Worse tradeoff.

### 5.4 Single-instance behavior in dev

The launcher's named-pipe bind is per-data-dir. Dev mode uses `~/.agentmux-dev` (a single data dir). Implication: **a second `task dev` run will hit `ERROR_ACCESS_DENIED` on the pipe and forward `open_new_window` to the existing instance** (per `main.rs:262-302`).

This is actually correct behavior — accidentally running `task dev` twice today silently produces two instances racing on the CEF cache. Forwarding to the existing instance is strictly better. No spec change needed, but call this out in the PR description so reviewers aren't surprised.

If a developer genuinely needs two parallel dev instances (e.g. testing multi-window UX without packaging), provide:

```sh
task dev:standalone   # legacy behavior: invokes agentmux-cef directly
```

implemented as a copy of `dev:serve` with the launcher invocation reverted to the direct host call. Use cases: bisecting the no-launcher fallback path, testing host-only changes faster.

### 5.5 IPC port file location

Host writes `<data-dir>/ipc-port` (per `cef/main.rs:233-244`) for the launcher's `forward_open_new_window` to read. With the launcher in the loop, the env var `AGENTMUX_DATA_DIR` is set by the launcher (`main.rs:511` via `paths.common.to_env_vars()`), so host writes to the launcher-shared dir. **No change needed** — the existing code path Just Works once the launcher is invoked.

### 5.6 `authkey.dev` for test harnesses

`cef/main.rs:336-342` writes `authkey.dev` when `AGENTMUX_DEV=1` is set. The current Taskfile sets `AGENTMUX_DEV=1` inline on the host launch (line 524-529 comment) because go-task's top-level `env:` block doesn't propagate. With the launcher in the loop, `AGENTMUX_DEV=1` must be:

1. Set on the launcher invocation (it's inherited by child processes via `tokio::process::Command::spawn`).
2. The launcher does **not** strip `AGENTMUX_DEV` from the host's env — it just appends `AGENTMUX_BACKEND_*` etc. (see `main.rs:497-517`). So `AGENTMUX_DEV=1 ./agentmux-launcher.exe --url=...` propagates correctly.

**Verify in implementation**: tail `~/.agentmux-dev/authkey.dev` after a launcher-driven dev startup. If empty, the inheritance broke.

---

## 6. Implementation plan

### 6.1 Phase 1 — Windows-only integration (this spec's core)

1. **Restructure `dist/cef-dev/`** in `Taskfile.yml` `dev:serve` (Windows branch only — Unix stays on direct host invocation per §5.2).
2. **Copy `agentmux-launcher.exe`** from `target/release/` into `dist/cef-dev/` root.
3. **Copy `agentmux-srv-{version}-windows.x64.exe`** into `dist/cef-dev/runtime/` (otherwise `srv_spawner::spawn_srv` fails to find srv).
4. **Replace the final launch command** with the launcher invocation.
5. **Add `task dev:standalone`** as the escape hatch.
6. **Update `BUILD.md`** + `CLAUDE.md` (note that `task dev` now exercises the launcher).
7. **Smoke test:**
   - `tail -f ~/.agentmux/logs/agentmux-launcher.log` shows `starting`, `paths resolved`, `Job Object created`, `pipe bind OK`, `spawned CEF host pid=...`, `entering host + srv concurrent wait`.
   - `~/.agentmux-dev/launcher-events.log` is being written.
   - `~/.agentmux-dev/launcher-sagas.db` is created.
   - Killing the launcher PID terminates host + srv via `KILL_ON_JOB_CLOSE`.
   - Second `task dev` exits silently (forwarding) or shows the "already running" dialog.
   - **Splash screen** (AgentY PR #822) appears between launcher start and host first paint, then fades out via `on_load_end` → `SetEvent(AGENTMUX_SPLASH_EVENT)`. The splash is automatically exercised once the launcher is in the loop — no separate plumbing needed. Goal is **production parity** for the launcher-spawn → first-paint window; the dev-specific Vite-wait phase intentionally remains uncovered.

### 6.2 Phase 2 — Unix dev integration (deferred to Phase 7)

Tracked via launcher main.rs:153 comment ("Phase 7 covers cross-platform parity"). Out of scope for this spec.

### 6.3 Phase 3 — Tooling alignment

1. Update `scripts/install-linux-desktop.sh` invocation in `Taskfile.yml:500` to reference the launcher binary (only relevant after Phase 2 lands).
2. Consider whether `bundle:linux` should also produce a `runtime/` subdir so Linux dev can follow the same shape as Windows when Phase 7 lands.

---

## 7. Risks

| Risk | Mitigation |
|---|---|
| Launcher's single-instance forwarding surprises developers running `task dev` twice | Document in PR; provide `task dev:standalone` |
| `AGENTMUX_DEV=1` env inheritance breaks `authkey.dev` write | Smoke test step in §6.1; if regression, set `AGENTMUX_DEV=1` explicitly in the launcher's `host_cmd.env(...)` block (still under launcher control) |
| Extra dev-startup latency from saga log open + IPC pipe bind | Measured: SQLite open + named-pipe bind ≪ 500 ms on a warm cache; acceptable |
| `dist/cef-dev/runtime/` deeper layout breaks dev tools that grep for `dist/cef-dev/agentmux-cef.exe` | Audit before merge: grep for `dist/cef-dev/agentmux-cef` across scripts/, docs/, CI configs |
| Vite hot-reload feedback loop interrupted by launcher's Job Object KILL_ON_JOB_CLOSE on file rebuild | The Taskfile already kills Vite via `trap` on exit; no change to that contract |
| Launcher tries to write `launcher-sagas.db` etc. to the dev data dir but fails (permissions / disk-full) — currently fatal at `main.rs:344-353` | Acceptable — same failure mode as production. Surfaces real bugs. |

---

## 8. Open questions

1. **Should the launcher gain a `--dev-flat` flag** for diagnosing the no-launcher path *without* the layout reshuffle? Considered but rejected in §5.3. Revisit if dev iteration on the launcher itself becomes painful.
2. **Should `task dev:standalone` write a banner** ("WARNING: launcher bypassed — Phase B features unexercised") to discourage accidental long-term use? Probably yes — add `echo` in the Taskfile cmd.
3. **Does the launcher's `vacuum_older_than` at startup add unwanted churn to dev?** Default retention is 7 days; dev sessions are short. Worst case: cheap SQL. Not worth gating.

---

## 9. Decision

Recommended: **§5.2 — reshape `dist/cef-dev/` to mirror the production layout, invoke the launcher.** Zero launcher code changes, ~15-line Taskfile change, escape hatch via `task dev:standalone`, no Unix work in this phase.

Approver sign-off below before implementation begins:

- [ ] AgentX (author)
- [ ] (Reviewer)
