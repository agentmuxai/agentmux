# STATUS — 2026-09-22 renderer deadlock: fix shipped, what remains (handoff)

**Status:** living — handoff doc for the Agent3 session that diagnosed the freeze; update as the follow-ups in §5 land.
**Date:** 2026-09-23
**Author:** Agent3 (Claude Fable 5.1), written at the operator's request before a model switch.
**Canonical RCA:** `docs/incident/INCIDENT_2026_09_22_RENDERER_MAIN_THREAD_DEADLOCK_ON_CHROMIUM_LOCK.md` (merged, #3549). This doc is the *state* on top of it, not a second copy of the analysis.

---

## 1. One paragraph

On 2026-09-22 22:42 the operator's v0.56.12 main window froze permanently. The renderer's Compositor thread self-deadlocked on the global mutex of Chromium's raw_ptr InstanceTracer (re-entered through GWP-ASan's contended-lock metrics timer); every other renderer thread then queued behind it; srv, host and launcher stayed healthy so every health signal was green. The tracer was on only because `scripts/cef-build/args-windows.gn` carried `enable_backup_ref_ptr_instance_tracer=true` (copied from a generated args.gn in #2308, 2026-07-27); upstream CEF's `tools/gn_args.py` sets it for every non-debug Windows build. Fix: flag off, Windows CEF rebuilt, published as `agentmuxai/cef` release `cef-windows-x86_64-152.0.7977.83-r2`, pinned on main by #3561. macOS/Linux were never affected (flag absent, Chromium default false, upstream only forces it on Windows).

## 2. What is DONE (all merged / published, verified)

| Item | Where | State |
|---|---|---|
| RCA + recovery procedure | `docs/incident/INCIDENT_2026_09_22_...md` | merged in #3549 (2026-09-23 07:04Z) |
| `args-windows.gn` tracer `true` → `false` | #3549 | merged |
| Windows CEF rebuilt with tracer off | `~/cef-build/chromium_git/chromium/src/out/Release_GN_152` | done 02:15 local; 34,869 steps, 0 errors; buildflag header reads `(0)`; cdb finds no `InstanceTracer` symbols in the new DLL; cefsimple boots |
| Release `cef-windows-x86_64-152.0.7977.83-r2` | `agentmuxai/cef` (author agent3-workflow[bot]) | runtime zip 192 MB + `libcef.dll.pdb-152.0.7977.83-r2.xz` 598 MB + `SHA256SUMS.txt` |
| Symbols for the runtime users run TODAY | `agentmuxai/cef` tag `cef-windows-x86_64-152.0.7977.83` | `libcef.dll.pdb-152.0.7977.83-shipped.xz` 604 MB attached |
| Pin bump (`fetch-patched-cef-windows.sh`, `release.yml`), tracer=false guards in `args-darwin.gn` + Linux `args.gn`, build-doc §7a | #3561 | merged (2026-09-23 12:23Z), all checks green |
| Pin validated | `gh release download` of the pinned tag/asset → libcef.dll byte-identical to the rebuilt one | verified |

**Users get the fix in the next portable/release built from main after #3561.** Any 0.56.x portable built before that still carries the tracer and can freeze again at random.

## 3. Evidence and artefacts (host `narko`, Agent3 workspace `~/.agentmux/agents/agent3-0630k/scratch/`)

- `renderer-52384-frozen.dmp` — 682 MB full dump of the frozen renderer, taken before it was killed. Reproduction artefact for any upstream report.
- `stack-52384.txt`, `stack-28648.txt`, `stack-53396.txt`, `stack-47672.txt` — raw cdb `~*k` snapshots (renderer, a pool renderer, browser process, GPU process) while frozen.
- `all-threads-symbolized.txt` — every renderer thread symbolised with the real PDB.
- `symbols-152.0.7977.83-tracer-on/` — `libcef.dll` + `libcef.dll.pdb` + `args.gn` matching the *shipped* runtime (copied before the rebuild overwrote the out dir). Same PDB is now on the GitHub tag.
- `release-r2/` — the published artefacts and notes.
- `cdp-eval.mjs`, `cdp-call.mjs`, `cdp-wsframes.mjs` — Chrome DevTools Protocol probes (renderer liveness test, browser-level calls, WebSocket frame watch). The host exposes `--remote-debugging-port`; `Runtime.evaluate` timing out on the main page is the decisive hung-renderer test.
- `cef-rebuild.log` — the ninja log.

Symbolising this or any future dump: `cdb -z <dmp> -y <dir containing the matching libcef.dll.pdb>`. **Never** point it at upstream CEF symbol packages — the fork binary has a different link order and every name comes back wrong (verified: export deltas range from +0x91430 to −0x3c93200).

## 4. Gotchas learned (save the next person hours)

- `out\Release_GN_x64` on the build machine is a **staged runtime copy**, not a build tree. The real incremental tree is `out\Release_GN_152` (has `build.ninja`, `obj\`, `args.gn`). The configure script hardcodes `Release_GN_x64`; running it would start a cold 3–6 h build. Check for `build.ninja` first.
- `gn gen` needs `DEPOT_TOOLS_WIN_TOOLCHAIN=0`, `vs2022_install="C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"`, and `C:\depot_tools` first on PATH; without the second it fails with "No supported Visual Studio can be found".
- Changing this flag rebuilds everything (the header is included everywhere): 2h18m on the 9950X3D at `-j 20 -l 28`, ~33 GB RAM free throughout.
- `build-windows.yml` resolves the **newest** `cef-windows-x86_64-*` release by publish date when given no tag and downloads `*.zip`; a release must contain exactly one zip, and any extra asset (PDB) must not be a `.zip`.
- Agent credentials: the Agent3 GitHub App token CAN create releases on `agentmuxai/cef` even though `GET repos/agentmuxai/cef` reports `push=false` for it. The `.gh-env.sh` PAT in the Agent3 workspace is dead (401) and the `gh-token-agent3` / `gh-token-genericagentx` secrets no longer exist. `scripts/gh-agent.sh` stops at the first working tier, so for cross-repo writes mint the token directly: `python scripts/github-app-token.py agent3 agentmuxai`.
- `scripts/cef-build/fetch-patched-cef-windows.sh` bails on `gh auth status` failing, and `gh auth status` exits non-zero in any shell that sees AgentY's stale keyring entry — even with a valid `GH_TOKEN`. Environmental; the pin itself is fine.
- Git Bash mangles `taskkill /PID` and relative paths passed to `gh`; use PowerShell `Stop-Process` and `cygpath -w` absolute paths.
- The srv warning `ws egress lane full … consumer stalled` means the frontend stopped reading; it is a symptom of a dead renderer, not an srv fault (contrast `INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`, where srv itself was wedged with the same surface signature). Distinguish them with the DevTools liveness test above.

## 5. What is NOT done (proposed; item 1 now done)

1. **DONE 2026-09-23** (branch `agent3/cef-renderer-unresponsive-recovery`): `client/unresponsive.rs`, wired in `handlers.rs`; the spec's §8.1 is now active. Original proposal kept below for context. **Auto-recover a hung renderer in the host** — highest value. `ImplRequestHandler::on_render_process_unresponsive(browser, callback)` exists in the `cef` 152 crate (and since 148); `agentmux-cef/src/client/handlers.rs` implements only `on_before_browse` and `on_render_process_terminated`. Plan: log at `crash` level, wait one grace interval, call the callback's terminate → lands on the existing recovery page (`client/recovery_pages.rs`); optionally auto-Reload. Design home: `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md` §8 ("Responsiveness"), still Draft — promote that section rather than write a new spec. Tracking issue #942. **Was never blocked on CEF work.**
2. **srv-side stalled-frontend signal** — N consecutive seconds of `egress lane full` on a connection → event to the host over the srv pipe → same terminate-and-reload. Covers the no-click case Chromium's own detector cannot see. The launcher's `ui_liveness.rs` probe cannot help here by design (it proves the browser UI thread pumped, which was fine).
3. **GN-args denylist** in `configure-cef-build-windows.ps1` / `configure-cef-build.sh` (and the macOS equivalent): refuse `gn gen` with `enable_backup_ref_ptr_instance_tracer`, `dcheck_always_on`, `is_debug`, `enable_backup_ref_ptr_slow_checks`, `is_asan` set to true.
4. **Upstream reports**: CEF (test-only flag forced on in Windows release builds via `tools/gn_args.py`), Chromium (`InstanceTracer` storage allocation re-enters the tracer via GWP-ASan + `LockMetricsRecorder`). Dump + symbolised stacks ready.
5. **Symbol store** (nice-to-have): `symstore.exe add` into the existing S3 artifacts bucket so `_NT_SYMBOL_PATH` resolves fork PDBs automatically instead of per-tag downloads.
6. Housekeeping: delete the dead `~/.agentmux/agents/agent3-0630k/.gh-env.sh`; fix/remove the fetch script's `gh auth status` precheck or make it token-aware; four orphan `wt-*` worktrees in AgentY's workspace (AgentY is on it).

## 6. Coordination state

- AgentY (same host) merged #3544, #3542, #3547 (cef-dll-sys pin → `agentmuxai/cef-rs`), #3551, and has #3554 auto-merge armed; none touch the files above. AgentY has the PDB-per-release recommendation and is passing it to the operator as done-by-#3561.
- ReAgent reviewed and approved both #3549 and #3561.
- The Windows handle-count srv tests (`sysinfo`, `fs_watch::pool`) fail on this machine only while a Chromium build is running; green again now.
