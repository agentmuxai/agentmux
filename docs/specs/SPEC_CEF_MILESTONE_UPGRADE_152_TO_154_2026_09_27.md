# CEF milestone upgrade: 152 (7977) → 154 (8037), all three platforms — in two days

**Author:** AgentY (narko), at operator request
**Created:** 2026-09-27
**Status:** implemented — shipped 2026-09-30: consumer PR #3992 merged 06:51:13 UTC and the three runtimes were published 06:51:19–22 UTC (tags on agentmuxai/cef 660112374b79). Also #3987 (prerequisite), #3989 (spec), #3991 (Linux args), agentmuxai/cef#10, agentmuxai/cef-rs#1; `agentmuxai/cef` default branch is now `8037`.
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
     A clean compile does not prove GPU works. On **Windows and Linux** run
     **`scripts/verify-angle-libs.sh`** (the ANGLE export check — `eglGetProcAddress`/`glGetString`
     in the separate `libEGL`/`libGLESv2`) on each built runtime and on the packaged app, as
     `Taskfile.yml` already does at package time. On **macOS** that script proves nothing: there are
     no separate ANGLE libraries, and it skips absent ones and still prints success. Instead `nm` the
     framework binary for the `angle::`/`rx::`/`DisplayMtl` symbols (148 → 152 spec, the macOS ANGLE
     finding — 152 had thousands of each); zero means no usable ANGLE. All three: plus a live
     `chrome://gpu` check. The §7.2
     probes check our carry-set symbols (`BeginWindowDrag`, …) in the main CEF library, not ANGLE.
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
9. **Agents publish through their GitHub App — no operator step.** *(Corrected 2026-09-29, after
   the operator asked whether publishing goes through CI. This trap used to say "agent identities
   have no write access to `agentmuxai/cef` releases (r2 notes)". That was written before agents moved
   to GitHub Apps. The r2 release it cites was
   itself published by an agent App. Check it with
   `gh-agent api repos/agentmuxai/cef/releases --jq '.[0:2][] | [.tag_name, .author.login]'`:
   `cef-windows-x86_64-152.0.7977.83-r2` → `agent3-workflow[bot]`, 2026-09-23.)* Each builder uploads its
   runtime as a **draft** from its own machine with `gh-agent release create --draft`, and the
   upgrade's manager publishes the three drafts (Day 2 step 4). A draft is not public and no build
   resolves it (step 2).

## 4. Plan

### Before Day 1: make "blank" mean "pinned" (prerequisite, merged before anything is published)

Every `build-*.yml` (Windows, Linux, macOS) resolves an empty `cef-runtime-tag` to the most
recently *published* `cef-<platform>-*` release (`gh release list`, sorted by `publishedAt`;
prereleases included). Two kinds of caller hit that path:

- `ci-nightly-artifacts.yml`, which passes `cef-runtime-tag: ""` for all three platforms;
- a direct `workflow_dispatch` / `repository_dispatch` of any `build-*.yml` with a blank tag.

Once a 154 release is published, any of them would pair the 154 runtime with `main`'s 152
binding. Windows then fails its version guard; Linux and macOS have no equivalent major-version
guard and can emit mismatched artifacts. Fix it at the one place they share: **change the
blank-tag resolution in each `build-*.yml` to the committed pin** instead of "latest published".
Move the per-platform values out of `release.yml`'s `cef-runtime-pins` job into **one shared pin
file** that `release.yml` and all three `build-*.yml` read — not per-workflow literals, which would
be four more places for the consumer PR to miss (Day 2 step 3 moves that file). The same PR
updates the docs that name `release.yml` as the pins' home — `CEF_FORK_MAINTENANCE.md` §8 (its
text and its `sed` command for reading the live values) and `README.md`'s pin note — so the next
upgrade doesn't edit the wrong file. That's the fix #3086 made for releases, extended to
every entry point. After it, publishing a runtime changes nothing until the consumer PR moves the
pins, and a deliberate non-pinned build has to name its tag.

**Known limitation — refs cut before this prerequisite.** A `workflow_dispatch` runs the workflow
*of the ref it is dispatched from*, not `main`'s (`release.yml` documents the same behavior), so a
blank-tag dispatch from a tag or branch older than this prerequisite still resolves to "latest
published". That is true of every runtime release, not just 154, and can't be fixed retroactively
in old refs. Day 2 step 4's order (merge, *then* publish) keeps it from mattering during this
upgrade: until the drafts are published, "latest published" is still 152, matching old refs' 152
code. Afterwards, rebuild an old ref only with an explicit `cef-runtime-tag`.

### Day 1 — morning: source (one owner, ~3 h)

Both forks' changes land as **reviewed PRs**, not direct pushes. The `cef` fork's PRs into `7977`
were reviewed by ReAgent and Codex; `cef-rs` has never had a PR, so the operator confirms both apps'
repository access covers it (org settings → GitHub Apps) before Day 1 — its change decides the ABI.

1. `agentmuxai/cef`: create integration branch **`8037`** from upstream 8037 (564dd6c4).
2. On a work branch off it, port the carry-set, one commit per item (`agentmux: port <item> to
   8037 (Chromium 154)`): the 18-file Layer B delta by per-file 3-way merge; the 3 Layer A patches
   registered in `patch.cfg`; `added=15400` + `version_manager.py -u` (trap 1). Open it as a PR
   into `8037`.
3. `agentmuxai/cef-rs`: base branch `agentmux/154` cut from `cef-v154.2.0+154.0.28`; a work branch
   off it cherry-picks bfeae80 and 9b0abfe; open it as a PR into `agentmux/154`.
4. Run the §5 carry-set gate (**21/21 `OK`**) and step 5's Cargo checks on the PR heads while
   review runs. **Merge both PRs, then record the SHAs from the merged `8037` and `agentmux/154`
   heads and re-run the gate and Cargo checks on exactly those** — an amended or squash-merged PR
   changes the SHA, and the pre-review one would bypass the review fixes. Those two final SHAs are
   what every platform builds and what the consumer PR pins.
5. `agentmux` (local only, not merged): `cef = "154"` + the new `[patch.crates-io]` rev;
   `cargo check --workspace` must be clean (§2 predicts no API fallout). **Also, on Linux:**
   `cargo check -p agentmux-cef --features patched-libcef` against the new binding rev. The
   `begin_window_drag` field access in `ui_tasks/drag.rs` compiles only under that feature, which is
   default-off, on Linux — without this, a missing or mis-generated 154 slot passes the workspace
   check and surfaces only after the expensive builds.

#### Day 1 results (2026-09-29)

- **Prerequisite** merged: agentmuxai/agentmux#3987 (pins in `scripts/cef-build/cef-runtime-pins.sh`);
  the drift reporter reads that file (updated in the private infrastructure repo, deployed).
- **`agentmuxai/cef` `8037`** cut from upstream **682c378d7**, not 564dd6c4: four later upstream
  fixes, same `refs/tags/154.0.8037.58`. Port PR #10, one commit per item, all 18 Layer B files
  3-way merged with no conflicts. **Build SHA: `660112374b790582d63a356da633fb2cb14b7538`**; gate
  21 OK / 0 MISS on it.
- **`agentmuxai/cef-rs` `agentmux/154`**: PR #1 (the two cherry-picks, plus an `offset_of!`
  assertion pinning `begin_window_drag` at 888). **Binding SHA:
  `9568a644ce2fd471d1f8812d953228bfec28c4fc`**; `cargo check --workspace` clean with `cef = "154"`.
  The Linux `--features patched-libcef` check runs on charlie.
- **API hashes:** after `version_manager.py -u` (run after patching, **before** ninja), expect
  "27/28 versioned match" plus "Hashes for version 15400 do not match". That single mismatch is
  expected: 15400 is frozen upstream and `BeginWindowDrag` is added to it. 152 shipped the same
  way on 15200 (55edc030's "26/27").

### Day 1 — afternoon → Day 2 morning: three builds in parallel (3 owners, 3–6 h each)

| Platform | Owner (did 152) | Machine | GN args | Watch for |
|---|---|---|---|---|
| Windows x86_64 | Korp | narko | `args-windows.gn` | `use_static_angle=false`, tracer off, exclude `installer_tests`. **Disk: narko has 82 GB free, about 40 GB short of the ≥120 GB a cold build needs** (open question 2). The 152 checkout at `C:\Users\asafe\cef-build` can be synced forward instead of recloned, which may need less — confirm before starting |
| macOS arm64 | Clare | Clare's Mac | `args-darwin.gn` | keep ANGLE static (no override), `dcheck_always_on=false`, hermetic Xcode pin |
| Linux x86_64 | Opaz | charlie | `args.gn` | `use_static_angle=false`, keep `libcef.so` **unstripped** (verify-cef-patch reads `.symtab`; corrected 2026-09-29), `-codecs` |

Before starting, each owner confirms ≥120 GB free and ≥32 GB RAM (a build that dies at hour five
for lack of disk is the expensive failure). Warm-cache rebuilds after a patch tweak are 5–30 min.

### Day 2: verify, publish, switch (all, then one owner)

1. **Per platform, §7** of the maintenance doc: `patcher.py` sanity (§7.1), symbol probes with full
   `nm` (§7.2) on macOS and Linux, and functional checks (§7.3). **On Windows the release
   `libcef.dll` carries no local symbols** — `dumpbin /symbols` on it finds nothing, healthy or
   not (`build-patched-cef-windows.md`, "Verifying `BeginWindowDrag` landed"). Probe the matching
   `libcef.dll.pdb` from the same `out/` directory instead (a PDB-aware dumper such as
   `llvm-pdbutil`), first confirming it finds a known-present symbol like `IsUniqueForCEF` so an
   empty result can't pass; for `BeginWindowDrag`, also check the generated `window_cpptoc.cc`
   slot as that doc describes. §7.3: boot + `chrome://gpu` GPU on; title-bar right-click;
   window transparency; H.264/HEVC playback; no -67030 on macOS 26; no DCHECK on drag/close.
   **Linux native drag must visibly move the window** (title bar and a floating window) in the
   packaged build, which enables `patched-libcef`. The size-mismatch fallback in
   `ui_tasks/drag.rs` (warning, no-op) is a **failure** here, not a pass: it's what an ABI mismatch,
   a null slot or a missing feature looks like, and it leaves users unable to drag.
   **The §7.2b differential compile (two `OK` lines) is a macOS-only gate.** Under
   `use_thin_lto=true` (Windows, Linux) an isolated single-TU recompile yields LLVM bitcode, not the
   machine code the ThinLTO backend links, so `cef-verify-patches.sh` cannot pass there regardless of
   the patches (148 → 152 spec, the 2026-09-15 status note; its default pairs also include the
   macOS-only `mach_port_rendezvous_mac.o`). On Windows and Linux the no-symbol patches are verified
   by the §7.1 patcher log (each registered patch reported applied) plus their §7.3 behavior —
   transparency for `rwhv_background_opaque_check`, title-bar right-click on Linux — as for 152.
   Giving those two platforms a real differential mechanism stays open (tracked in the 152 spec).
2. **Stage, don't publish yet.** A GitHub *draft* release is not published: it has no
   `publishedAt`, isn't public, and `gh release list` resolution can't select it — publishing happens
   only in step 4, behind that step's gate (the blank = pinned prerequisite confirmed merged). Upload
   the three runtimes as **draft** releases on
   `agentmuxai/cef` (each builder, via `gh-agent`, trap 9): `cef-windows-x86_64-154.0.8037.58`,
   `cef-macos-arm64-154.0.8037.58-codecs`, `cef-linux-x86_64-154.0.8037.58-codecs` — one tag scheme
   (`cef-<os>-<arch>-<chromium>[-codecs][-rN]`). A draft has no `publishedAt`, so even an unpinned
   nightly wouldn't pick it; it isn't public; and a problem found in step 4 can still be fixed without
   an immutable bad tag. **A corrected runtime gets a new `-rN` tag, never new bytes under the same
   tag:** all three `build-*.yml` cache the runtime by tag alone and skip the download on a hit, and
   Linux and macOS have no asset-hash check, so a replaced asset under an old tag can pass the gate
   on cached, stale bytes while different, untested ones get published. Delete the superseded draft
   and point the consumer PR's pins at the new tag.
3. **One consumer PR in `agentmux`** (trap 8): `agentmux-cef/Cargo.toml` `cef = "154"`; root
   `[patch.crates-io]` → the new `agentmuxai/cef-rs` rev; **`Cargo.lock`** regenerated and committed so
   it records `cef`/`cef-dll-sys` 154.2.0+154.0.28 at that rev (today it pins 152.1.0 and `9b0abfe`;
   left stale, the next Cargo run may resolve an unverified 154 crate); **the shared pin file from
   the prerequisite** (read by `release.yml` and every `build-*.yml`'s blank-tag path) — all three
   tags together; left on 152, nightly and blank-tag builds keep fetching 152 runtimes against the
   154 binding; **`scripts/cef-build/windows-runtime-pin.sh` — all three of its values**
   (`CEF_WINDOWS_RELEASE_TAG`, `CEF_WINDOWS_ASSET`, `CEF_WINDOWS_LIBCEF_SHA256` of the new
   `libcef.dll`). That file, not `fetch-patched-cef-windows.sh` (which only sources it), is the
   Windows pin local and package builds use: leave it on 152 and they fetch the 152 runtime and fail
   the version guard, while a 154 runtime fails `verify-cef-runtime-windows.sh`'s SHA check. Also
   the Taskfile CEF tiers if they name a version, `docs/cef-build/*` version references, and the
   public ones: `README.md` (the architecture image's alt text and the "Desktop: CEF 152" line) and
   the version text inside `assets/architecture.svg` (edited in place — it has no generator; #3270
   did the same for 148 → 152).
4. **On the consumer branch**, a packaged build on each platform — using the locally built runtime
   (or the draft asset) — runs the §7.3 checks end to end. **Gate: the blank = pinned prerequisite
   is merged on `main`**; if not, stop. Only when all three platforms pass and that gate holds:
   **merge the consumer PR, then publish the three drafts immediately** — in that order. Between the
   two, `main` names 154 tags that aren't published yet, so any build of `main` **fails loudly**
   (it can't download the runtime) instead of producing a 154-runtime/152-binding mismatch, and
   "latest published" is still 152 for any old-ref dispatch (see the known limitation above). A
   failed nightly for a few minutes is the intended cost; a mismatched artifact is not possible.
5. After: switch `agentmuxai/cef`'s default branch to `8037` (needs repo admin — flag it, §6.8).

### Rollback

Release tags are immutable: revert the consumer PR to restore the 152.0.7977.83 pins (and the
`-r2` Windows tag). Keep the 152 releases permanently.

## 4a. Outcome (2026-09-30)

Two days, as planned. Windows (Korp), macOS (Clare) and Linux (Opaz) built one commit,
`agentmuxai/cef` `8037` @ `660112374b79`, and all three passed §7 and the step 4
packaged-app checks on the consumer branch before the merge. The operator did the
hands-on checks on macOS and Linux. Publishing followed the merge by about 8 seconds.

What cost time, none of it source or patch work (the lessons are now in
`CEF_FORK_MAINTENANCE.md` §6.1):
- **Linux:** the fontconfig bindgen `-Dwarnings` failure (#3991); stale Dawn Go files from
  syncing the 152 tree in place; the charlie VM halting at 09:48 UTC when gamerlove's
  NVIDIA driver crashed (about 3 h); and a packaged-app window that stayed invisible with
  the VM's 3D off (a GPU-tier bug, `docs/retro/RETRO_INVISIBLE_WINDOW_ON_DEAD_GPU_2026_09_29.md`,
  fixed by #3995).
- **Windows:** a `subst` drive broke a TypeScript step; `-j 20` exceeded narko's commit limit.
- **macOS:** the first §7.3 pass used `main`'s 152 binding; it was repeated on #3992.
- **Coordination:** Korp sat unreachable for an hour after a deferred config restart left
  it with no process (`docs/retro/RETRO_DEFERRED_RESTART_NEVER_RESPAWNS_2026_09_29.md`,
  fixed by #3990), and the relay briefly stopped delivering to Opaz.

**Known issue carried forward:** Linux window transparency fails, identically on the
152-based 0.58.2, so it is an earlier regression, not 154. The operator chose to fix it
after 154.

## 5. Risks to the two-day estimate

| Risk | Likelihood | Mitigation |
|---|---|---|
| A build fails late (disk, flag, new Chromium compile break like `installer_tests`) | medium | disk check first; start all three early on Day 1; warm-cache fixes are minutes |
| A silent runtime regression (the ANGLE class) | medium | the trap 6 ANGLE gates — `verify-angle-libs.sh` on Windows/Linux, the framework `nm` probe on macOS — plus live `chrome://gpu`, not compile success (§7.2 checks carry-set symbols, not ANGLE) |
| Chromium 154 behavior changes in the frontend | low–medium | two milestones, not four; the §7.3 matrix plus a normal release smoke test |
| Publishing stalls after the consumer merge | low | the manager publishes all three drafts right after merging (agent App, no human step, trap 9); if a publish fails, revert the merge (`main` builds fail until one of the two happens) |
| Only two owners available | — | Windows and Linux on narko/charlie in parallel; macOS on Day 2 (adds ~half a day) |

## 6. After 154: make the next one routine

Chromium ships a milestone about every four weeks, and only the newest CEF branch gets fixes, so
**155 lands in late October** (and 156 around late November). This upgrade should leave behind: a script for the Layer B 3-way port
and patch registration (steps 1–3 above), the §2 header comparison as a script, and the
version drift report's CEF rows as the trigger. Tracked separately.

## 7. Open questions for the operator (answered 2026-09-29)

1. Owners: Korp (Windows), Clare (macOS), Opaz (Linux) again, or others? **Yes, those three;
   AgentY is manager lead.**
2. Disk: narko has 82 GB free, about 40 GB short of the ≥120 GB a cold build needs. **Resolved
   per host, by the operator:**
   - **narko:** a 2026-09-28 cleanup of idle agents' Rust `target/` dirs left ~880 GB free. The
     Windows build needs no other change.
   - **charlie:** 23 GB free. The operator approved archive-then-delete of the 152
     `out/Release_GN_x64`, for this build only. Opaz archived the 152 debug info
     (`~/cef-152-symbols-linux.tar.zst`, 4.0 GB, 46,373 entries, checked with `tar -tf`), deleted
     `out/` (95 GB), and left charlie with 119 GB free.
   - **Deleting a previous milestone's `out/` stays an operator decision on each host.** Archive its
     unstripped `libcef` and debug info first: they are the only symbols matching the shipped
     runtime.
3. Who publishes the three releases, and when on Day 2? **The builders upload drafts; AgentY
   publishes them right after the consumer PR merges (trap 9).**
