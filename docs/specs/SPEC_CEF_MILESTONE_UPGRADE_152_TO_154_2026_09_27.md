# CEF milestone upgrade: 152 (7977) → 154 (8037), all three platforms — in two days

**Author:** AgentY (narko), at operator request
**Created:** 2026-09-27
**Status:** proposed
**Executes:** `docs/cef-build/CEF_FORK_MAINTENANCE.md` §6 (upgrade runbook), §5 (carry-set gate),
§7 (artifact verification), §8 (release pinning), §9 (checklists). This spec does not restate them;
it adds what is specific to 154, the schedule, and the traps 148 → 152 already hit.
**Predecessor:** `docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` (took 2026-09-07 →
09-15 for the pins, 09-23 for the tracer-free Windows `-r2`).

## 1. Why now, and why straight to 154

| | Ours | Upstream |
|---|---|---|
| Milestone | 152 (CEF branch **7977**) | **154** (branch **8037**, stable since 2026-09-22) |
| Runtime (all 3 platforms) | 152.0.7977.**83** | 7977 tip **.140** (2026-09-24); 8037 tip **154.0.8037.58** / CEF 154.0.28 |
| Binding (`cef-dll-sys`) | 152.1.0+152.0.6 via `agentmuxai/cef-rs@9b0abfe` | 154.2.0+154.0.28 |

The version drift report flags the three runtimes as **error** (57 Chromium patch builds behind).
CEF's 7977 branch shipped its last build (.140) two days after 154 went stable, and CEF generally
maintains only the current stable branch — so 152 is most likely finished receiving security
fixes. A .140 patch rebuild would be superseded within days; **go straight to 154** and skip it.
(If 154 slips past ~2 weeks, the .140 rebuild is the fallback — same branch, same patches, three
rebuilds, no porting.)

## 2. Recon already done (2026-09-27, dry runs — nothing built yet)

- **Rust side: nothing to port.** All 12 CEF headers behind the `cef::Impl*` traits `agentmux-cef`
  calls (`cef_auth_callback.h`, `cef_browser.h`, `cef_trace.h`, `cef_frame.h`, `cef_task.h`,
  `views/cef_{overlay_controller,panel,panel_delegate,view,view_delegate,window,window_delegate}.h`)
  and `cef_api_hash.h` are **byte-identical** between upstream 7977 and 8037.
  `include/internal/cef_types.h` differs by one versioned enum value
  (`CEF_CPAIT_WALLET_REMINDER_NOTICE`, `CEF_API_ADDED(15400)`).
- **Fork, Layer B (18 CEF-source files):** the fork's whole delta on `agentmuxai/cef` `7977`
  (fe7c8a3c) **3-way merges onto upstream `8037` (564dd6c4) with zero conflicts**.
- **Fork, Layer A (3 Chromium patches):** all three **apply to Chromium 154.0.8037.58** source,
  after CEF 8037's own `views_widget.patch` (the rwhv hunk at a +43-line offset):
  `agentmux_process_requirement` (macOS), `views_caption_rightclick_passthrough` (Linux),
  `rwhv_background_opaque_check` (cross-platform file; matters on macOS).
- **Binding fork:** both `agentmuxai/cef-rs` commits — bfeae80 (`begin_window_drag` appended to
  `_cef_window_t`, Linux x86_64 only, 888 → 896) and 9b0abfe (`links = "cef_dll_wrapper_agentmux"`)
  — **cherry-pick onto upstream `cef-v154.2.0+154.0.28` (61976b0) with no conflicts**. In 154,
  `_cef_window_t` is still 888 bytes and still ends with `get_runtime_style`, so the slot is at the
  same place.

So the cost is almost entirely the **three from-source builds and their verification**, which is
what the two-day schedule is built around.

## 3. Traps from 148 → 152 (do not rediscover them)

Sources: `CEF_FORK_MAINTENANCE.md`, the 152 spec, `agentmuxai/cef` PRs #7–#9 and its release notes.

1. **Bump the `added=` API annotation to 15400** on `BeginWindowDrag` and run
   `tools/version_manager.py -u` — a stale value fails the build at target 0
   (`cef_api_untracked.json` missing; that generated file is deliberately not committed). Only a
   compile caught it last time (55edc030).
2. **Port our delta, not a branch.** Per-file 3-way merges (base upstream 7977, ours `agentmuxai/7977`,
   theirs upstream 8037). Merging whole feature branches drags old CEF patches in.
3. **Every Chromium patch must be registered in `patch/patch.cfg`** (keep both entries when the
   tail conflicts) and use **no `a/`/`b/` prefixes** (`git apply -p0`). An unregistered patch is
   silently never applied — `rwhv_background_opaque_check` was registered only on a feature branch.
4. **Build from the integration branch `8037` only**, one recorded commit for all three platforms,
   and tag *that* commit (not a branch; `CEF_COMMIT_HASH` is Chromium's cut, not ours).
5. **Carry-set gate: all 21 probes `OK`** before building (§5). It proves presence, not
   correctness — the open Views-transparency issue (#3127) is not fixed by passing it.
6. **Build flags matter as much as patches** (`scripts/cef-build/args*.gn`):
   - `use_static_angle` is **per platform** — `false` on Windows and Linux (152 shipped stub ANGLE
     otherwise, #3172, #3229); macOS links ANGLE statically and must **not** override it.
     A clean compile does not prove GPU works: run the §7.2 symbol probes on *both*
     `libEGL`/`libGLESv2` and the main library, and a live GPU check.
   - `enable_backup_ref_ptr_instance_tracer=false` (the renderer deadlock that forced Windows `-r2`,
     #3561) — on all three, from the first build.
   - `dcheck_always_on=false` (macOS DCHECK crash on drag/close).
   - Keep PDBs/symbols; upstream symbols don't match our binaries.
7. **Windows: exclude `installer_tests`** from the ninja target list (fails under
   `is_official_build` + `NDEBUG`; testonly, never shipped — #3160).
8. **No per-platform staging.** Cargo allows one package per `links` value in the whole graph, so
   `agentmux-cef` cannot link 152 on one platform and 154 on another. The consumer switch is **one PR
   after all three runtimes exist**; the `cef-runtime-pins` job fails a release whose three tags
   disagree on milestone (by design).
9. **Publishing needs the operator's account** — agent identities have no write access to
   `agentmuxai/cef` releases (r2 notes). Plan for a human step per platform.

## 4. Plan

### Day 1 — morning: source (one owner, ~3 h)

1. `agentmuxai/cef`: create integration branch **`8037`** from upstream 8037 (564dd6c4).
2. Port the carry-set onto it, one commit per item (`agentmux: port <item> to 8037 (Chromium 154)`):
   the 18-file Layer B delta by per-file 3-way merge; the 3 Layer A patches registered in
   `patch.cfg`; `added=15400` + `version_manager.py -u` (trap 1).
3. Run the §5 carry-set gate: **21/21 `OK`**. Record the commit SHA — every platform builds it.
4. `agentmuxai/cef-rs`: branch `agentmux/154-begin-window-drag` from `cef-v154.2.0+154.0.28`,
   cherry-pick bfeae80 and 9b0abfe; record its SHA.
5. `agentmux` (local only, not merged): `cef = "154"` + the new `[patch.crates-io]` rev;
   `cargo check --workspace` must be clean (§2 predicts no API fallout).

### Day 1 — afternoon → Day 2 morning: three builds in parallel (3 owners, 3–6 h each)

| Platform | Owner (did 152) | Machine | GN args | Watch for |
|---|---|---|---|---|
| Windows x86_64 | Korp | narko | `args-windows.gn` | `use_static_angle=false`, tracer off, exclude `installer_tests`. **Disk: narko has 82 GB free, about 40 GB short of the ≥120 GB a cold build needs** (open question 2). The 152 checkout at `C:\Users\asafe\cef-build` can be synced forward instead of recloned, which may need less — confirm before starting |
| macOS arm64 | Clare | Clare's Mac | `args-darwin.gn` | keep ANGLE static (no override), `dcheck_always_on=false`, hermetic Xcode pin |
| Linux x86_64 | Opaz | charlie | `args.gn` | `use_static_angle=false`, strip `libcef.so`, `-codecs` |

Before starting, each owner confirms ≥120 GB free and ≥32 GB RAM (a build that dies at hour five
for lack of disk is the expensive failure). Warm-cache rebuilds after a patch tweak are 5–30 min.

### Day 2: verify, publish, switch (all, then one owner)

1. **Per platform, §7** of the maintenance doc: `patcher.py` sanity (§7.1), symbol probes with full
   `nm` (§7.2), and functional checks (§7.3): boot + `chrome://gpu` GPU on; native drag (Linux) or
   its documented no-op; title-bar right-click; window transparency; H.264/HEVC playback; no -67030
   on macOS 26; no DCHECK on drag/close.
   **The §7.2b differential compile (two `OK` lines) is a macOS-only gate.** Under
   `use_thin_lto=true` (Windows, Linux) an isolated single-TU recompile yields LLVM bitcode, not the
   machine code the ThinLTO backend links, so `cef-verify-patches.sh` cannot pass there regardless of
   the patches (148 → 152 spec, the 2026-09-15 status note; its default pairs also include the
   macOS-only `mach_port_rendezvous_mac.o`). On Windows and Linux the no-symbol patches are verified
   by the §7.1 patcher log (each registered patch reported applied) plus their §7.3 behavior —
   transparency for `rwhv_background_opaque_check`, title-bar right-click on Linux — as for 152.
   Giving those two platforms a real differential mechanism stays open (tracked in the 152 spec).
2. **Publish** three releases on `agentmuxai/cef` (operator account, trap 9):
   `cef-windows-x86_64-154.0.8037.58`, `cef-macos-arm64-154.0.8037.58-codecs`,
   `cef-linux-x86_64-154.0.8037.58-codecs` — one tag scheme (`cef-<os>-<arch>-<chromium>[-codecs][-rN]`).
3. **One consumer PR in `agentmux`** (trap 8): `agentmux-cef/Cargo.toml` `cef = "154"`; root
   `[patch.crates-io]` → the new `agentmuxai/cef-rs` rev; `release.yml` `cef-runtime-pins` — all
   three tags together; **`scripts/cef-build/windows-runtime-pin.sh` — all three of its values**
   (`CEF_WINDOWS_RELEASE_TAG`, `CEF_WINDOWS_ASSET`, `CEF_WINDOWS_LIBCEF_SHA256` of the new
   `libcef.dll`). That file, not `fetch-patched-cef-windows.sh` (which only sources it), is the
   Windows pin local and package builds use: leave it on 152 and they fetch the 152 runtime and fail
   the version guard, while a 154 runtime fails `verify-cef-runtime-windows.sh`'s SHA check. Also
   the Taskfile CEF tiers if they name a version, and `docs/cef-build/*` version references.
4. A packaged build on each platform runs the §7.3 checks end to end; then release.
5. After: switch `agentmuxai/cef`'s default branch to `8037` (needs repo admin — flag it, §6.8).

### Rollback

Release tags are immutable: revert the consumer PR to restore the 152.0.7977.83 pins (and the
`-r2` Windows tag). Keep the 152 releases permanently.

## 5. Risks to the two-day estimate

| Risk | Likelihood | Mitigation |
|---|---|---|
| A build fails late (disk, flag, new Chromium compile break like `installer_tests`) | medium | disk check first; start all three early on Day 1; warm-cache fixes are minutes |
| A silent runtime regression (the ANGLE class) | medium | §7.2 probes on both library candidates + live `chrome://gpu`, not compile success |
| Chromium 154 behavior changes in the frontend | low–medium | two milestones, not four; the §7.3 matrix plus a normal release smoke test |
| A human isn't available to publish | medium | schedule the operator's three publishes for Day 2 midday |
| Only two owners available | — | Windows and Linux on narko/charlie in parallel; macOS on Day 2 (adds ~half a day) |

## 6. After 154: make the next one routine

Chromium ships a milestone about every four weeks, and only the newest CEF branch gets fixes, so
156 lands in late October. This upgrade should leave behind: a script for the Layer B 3-way port
and patch registration (steps 1–3 above), the §2 header comparison as a script, and the
version drift report's CEF rows as the trigger. Tracked separately.

## 7. Open questions for the operator

1. Owners: Korp (Windows), Clare (macOS), Opaz (Linux) again, or others?
2. Disk: narko has 82 GB free, about 40 GB short of the ≥120 GB a cold build needs. Free the space, rely on syncing the existing checkout forward, or build Windows elsewhere?
3. Who publishes the three releases (operator account), and when on Day 2?
