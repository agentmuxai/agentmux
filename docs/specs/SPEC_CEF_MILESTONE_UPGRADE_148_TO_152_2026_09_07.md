# CEF milestone upgrade: 148 (7778) → 152 (7977), all three platforms

**Author:** AgentX
**Created:** 2026-09-07
**Status:** active — Phase A shipped 2026-09-08 (`agentmuxai/cef` PR #7,
plus the recon output `docs/reports/REPORT_CEF_UPGRADE_PHASE_A_RECON_2026_09_08.md`);
verdict is **go** on targeting 152 directly — patches #1/#2/#3 are verified
against real CEF/Chromium 152 source, and of the 18 fork-modified CEF files,
**16 are byte-identical upstream 7778↔7977** (mechanical copy) with only **2
genuinely drifted** (`include/internal/cef_types.h`,
`libcef/renderer/render_manager.cc`). Open: the macOS hermetic Xcode pin
(Phase D build-time check), and **patch #4 is only partially verified** — it is
five coupled CEF-side commits, not just the one Chromium-side file that was
test-applied (Codex, PR #3095). **Phase B's port is PARTIAL — 3 of 18 files
done, NOT ready for Phase D.** Branches `7977`,
`agentmux/7977-process-requirement`, `agentmux/7977-drag-rightclick-and-transparency`
exist. **Phases C–G not started; nothing is built or tested — `agentmuxai/cef`
has no CI.** The
recon **corrects two errors in §2's patch table** (marked inline below), makes
§4's Phase E work different from what's written there (see the callout in that
section — the un-pinned-CEF finding it describes was closed by #3086/#3085/#3089),
and surfaces one shipped-binary gap (§5 of the report): patch #1 was never
registered in `patch.cfg`, so it is absent from the shipped macOS binary —
fixed at source in `agentmuxai/cef` PR #7, still needs one macOS rebuild.
Verified 2026-09-08.
**Priority:** Medium-high — no active breakage, but we are four Chromium milestones behind and the gap grows by one milestone roughly every four weeks.

---

## 1. The ask, and what's actually true today

Upgrade the bundled CEF/Chromium runtime across Windows, macOS, and Linux. Two *separate* gaps exist and they have very different costs — conflating them is the main way this work gets mis-estimated:

| Gap | From → To | Nature | Cost |
|---|---|---|---|
| **Patch-level drift** (inside our own milestone) | `148.0.7778.180` → `148.0.7778.218` | Security/stability patches only, no API change | Rebase + rebuild. No source porting. |
| **Milestone drift** | CEF `7778` (Chromium 148) → CEF `7977` (Chromium 152) | Four milestones; CEF API surface moves | The real work — this spec. |

Verified upstream state (`chromiumembedded/cef`, read from each branch's `CHROMIUM_BUILD_COMPATIBILITY.txt` on 2026-09-07):

| CEF branch | Chromium | Note |
|---|---|---|
| **7778** | 148.0.7778.218 | our milestone; upstream tip is 38 patch versions ahead of our builds |
| 7827 | 149.0.7827.201 | |
| 7871 | 150.0.7871.252 | |
| 7922 | 151.0.7922.174 | |
| **7977** | **152.0.7977.83** | newest stable branch — the target |
| master | 152.0.7977.0 | |

Our pins: `agentmux-cef/Cargo.toml` → `cef = { version = "148" }`; root `Cargo.toml` `[patch.crates-io]` → `cef-dll-sys` from `AgentU-asaf/cef-rs` @ `515b3ac53c`; runtime binaries from `agentmuxai/cef` releases (`cef-windows-x86_64-148.0.7778.180`, `cef-linux-x86_64-148.0.7778.180-codecs`, `cef-macos-arm64-148.23.23-codecs`).

---

## 2. What we carry on the fork — the thing that makes this expensive

A milestone upgrade is not "bump a number." It is "forward-port every patch, rebuild three binaries, re-verify every feature those patches enable." Current inventory, from `docs/analysis/REPORT_CEF148_FORK_UPDATE_2026-06-02.md` and `agentmuxai/cef`'s branches:

| # | Patch | Upstream status | Used by | Port required? |
|---|---|---|---|---|
| 1 | `agentmux_process_requirement.patch` (`GetPeerValidationPolicy() → kNoValidation`, the -67030 renderer fix) | Not upstream | ~~All platforms~~ → **macOS only** ⁽¹⁾ | **Yes** — mandatory; ~~registered in `patch/patch.cfg`~~ → **was not registered or merged until 2026-09-08** ⁽²⁾ |
| 2 | `CefWindow::BeginWindowDrag()` (native HTCLIENT-region window drag) | Not upstream as of 148 — **re-check against 152** | **Linux only.** Windows uses its own Win32 path (`post_win32_begin_move`) and never calls this — `scripts/cef-build/args-windows.gn:4` states the patch "never [was] needed" there; macOS uses AppKit drag regions. Gated by the `patched-libcef` feature | **Yes, if Linux native drag stays in scope** — nothing else depends on it |
| 3 | HTCAPTION right-click fall-through to renderer | Not upstream as of 148 — **re-check against 152** | Title-bar right-click menus | Yes, if that UX is kept |
| 4 | Transparency cascade (RWHView/WebContents bg) | **Partial** upstream since 148 | Window opacity | Re-verify; may be droppable |

Plus **build flags, not patches** — version-controlled in *this* repo, not the fork:
`scripts/cef-build/args.gn` (Linux), `args-darwin.gn` (macOS), `args-windows.gn` (Windows). These carry `proprietary_codecs=true`, `ffmpeg_branding="Chrome"`, HEVC/AC3/EAC3/Dolby Vision enables, Widevine, and (critically) the `dcheck_always_on=false` requirement — a from-source `is_official_build=false` build defaults DCHECKs *on*, which crashes on drag/close (see `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md`).

**Re-checking #2, #3 and #4 against upstream 152 is the first task in this spec, because the answer can delete work.** Between 146 and 148, several AgentMux patches (`CefV8BackingStore`, `CefComponentUpdater`, `blink_ax_viewport_collapse`, the task-manager shutdown fix) were upstreamed and stopped being ours to carry. The same may have happened again across four milestones.

> **Phase A corrections (2026-09-08)** — full detail in
> `docs/reports/REPORT_CEF_UPGRADE_PHASE_A_RECON_2026_09_08.md`:
>
> ⁽¹⁾ Patch #1 touches `base/apple/mach_port_rendezvous_mac.cc`. Mach ports are
> Darwin-only and Chromium's `base/apple/` is not compiled on Windows/Linux, so
> this patch is **macOS-only** and cannot affect the other two platforms.
>
> ⁽²⁾ It was neither registered in `patch.cfg` nor merged into `7778` — it sat
> on an unmerged, diverged branch from 2026-06-02 until `agentmuxai/cef` PR #7
> (2026-09-08). That PR also fixed a defect that would have broken *every*
> source build from that branch: the patch's diff header used `a/`/`b/`
> prefixes, which CEF's `git apply -p0` patcher cannot resolve.
>
> **Patch #2 (`BeginWindowDrag`) is confirmed still NOT upstream at 152** —
> checked `include/views/cef_window.h` at upstream `7977` directly, so we must
> still carry it, and the `cef-dll-sys` binding patch is still required. But
> carrying it is nearly free: upstream never touched any of its three target
> files across four milestones (`cmp`-identical 7778 ↔ 7977), so the port is a
> copy rather than a forward-port. Done — see the report's §5b.
> Patch #3 **has since been test-applied** against real Chromium 152 source and
> applies cleanly, and patch #2's three target files are `cmp`-identical between
> 7778 and 7977. **Patch #4 is only partially verified** — its Chromium-side
> file applies cleanly, but its five-commit CEF-side transparency cascade
> (`SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md` §3) is not verified. See the
> report's §2.4/§2.5/§5b.

---

## 3. Decision: target 152 directly, not milestone-by-milestone

**Recommendation: go straight to 7977 (Chromium 152).** Rationale:

- The per-milestone cost is dominated by *rebuild* (3–6 h × 3 platforms), not by *porting*. Stepping through 149 → 150 → 151 → 152 multiplies the expensive part by four while only marginally easing the porting part.
- Our patch set is small (4 items, one mandatory) and touches stable CEF surface (`cef_window.h`, `window_impl.{cc,h}`, `window_view.cc`, `browser_view_impl.cc`).
- We have an ABI safety net, though a narrower one than "drag keeps working": `agentmux-cef/src/ui_tasks/drag.rs` compares the runtime `_cef_window_t.size` against the compiled binding's and **returns early with a warning** if they diverge, so an unpatched or mismatched runtime cannot read the extension slot (which would be UB). It prevents memory unsafety; it does **not** preserve the feature. On Linux the practical result is that native window drag becomes a no-op. Windows and macOS are unaffected either way — they never route through this patch (see §2, patch #2).

**Counter-case, stated honestly:** if the port hits a wall on 152, the fallback is to land on 7871 (150) — still three milestones of progress — rather than abandoning the effort. Decide this only if 152 porting exceeds ~2 days.

---

## 4. Work breakdown

### Phase A — Reconnaissance ✅ **COMPLETE 2026-09-08** (1 minor check deferred to Phase D)

Output: `docs/reports/REPORT_CEF_UPGRADE_PHASE_A_RECON_2026_09_08.md`.
**Verdict: go on the 152 target** — nothing found makes it harder, and 16 of the
18 fork-modified CEF files are byte-identical upstream. **Not a readiness
statement:** patches #1/#2/#3 are verified against real 152 source, **#4 only
partially** (its CEF-side cascade is unverified), and Phase B is 3/18 files. The
macOS hermetic Xcode pin also remains unconfirmed — a Phase D build-time check.
See the report's §2.4/§2.5/§5b.
Answers to the three tasks below:
(1) patch inventory corrected — see §2's callout; `BeginWindowDrag` confirmed
still not upstream at 152, so nothing was deleted. **Patches #1/#2/#3 are
verified against real 152 source** (#1/#3 by real `git apply -p0`; #2's three
target files `cmp`-identical between 7778 and 7977). **Patch #4 is only
partially verified** — its Chromium-side file applies, its CEF-side cascade does
not — report §2.4/§2.5/§5b. (2) **yes**,
`cef 152.0.0+152.0.5` is published; `begin_window_drag` is **not** in it, so
the binding fork is still needed. (3) Windows and Linux toolchain pins are
**unchanged** between the two Chromium tags; macOS's Xcode pin is
**unconfirmed** — verify at Phase D build time.

**Phase B's port is PARTIAL** — see the report's §5b. 3 of 18 fork-modified CEF
files are on the 152 branches, with `patch.cfg` registrations for the
`.patch`-file patches. Patch #2 needed no forward-porting (upstream never
touched its three files in four milestones), and 16 of the 18 files are likewise
byte-identical so they copy rather than port — but **2 drifted files need real
merge + compile work**, and patch #4's five-commit CEF-side cascade is not fully
ported or verified. A registration gap on `7778` (patch #4 registered only on
the build branch, not the integration branch) was found and not carried forward.
**Nothing is built — Phase D is the gate, and Phase B must finish first.**

*(original task list, for reference)*
1. Diff each of the four patches against upstream 7977; determine which are now upstream, which apply cleanly, which need real porting.
2. Check whether `cef-rs`/`cef-dll-sys` publishes a 152-compatible crate, and whether `begin_window_drag` is in its generated bindings (it wasn't for 148 — hence our `AgentU-asaf/cef-rs` patch fork).
3. Confirm the CEF 152 build toolchain requirements haven't moved (VS version on Windows, Xcode on macOS, sysroot on Linux).

**Output:** a go/no-go on §3's "straight to 152" decision, plus a corrected patch inventory. **This phase alone justifies its own PR** — it may materially shrink or grow everything below.

### Phase B — Fork source work (`agentmuxai/cef`)
Following the existing branch convention `agentmux/<milestone>-<feature>`:
1. Create `7977` (upstream mirror branch) in the fork.
2. Create `agentmux/7977-process-requirement` on upstream 152; port patch #1; register in `patch/patch.cfg` so `cef_create_projects.sh` applies it automatically.
3. Port surviving patches from `agentmux/7778-drag-rightclick-and-transparency` onto `agentmux/7977-drag-rightclick-and-transparency`.

Note: `agentmuxai/cef` has **zero CI** (verified: `actions/workflows` → `total_count: 0`). Nothing validates these branches automatically; a source PR there is reviewed by humans and proven only by an actual build in Phase D.

### Phase C — Rust binding
1. Bump `agentmux-cef/Cargo.toml`: `cef = { version = "152" }`.
2. Rebase the `cef-dll-sys` binding patch (`begin_window_drag` slot) onto the 152 binding; publish as `AgentU-asaf/cef-rs@agentmux/152-begin-window-drag`; update the `[patch.crates-io]` rev in root `Cargo.toml`.
3. Fix compile fallout from CEF API changes across four milestones — **the least predictable item in this spec**; no way to size it before Phase A.

> **Decision (2026-09-08): stage the rollout per platform instead of bumping the
> workspace-wide `cef` version all at once — Windows moves to 152 first;
> macOS/Linux stay on 148, in source and runtime both, until their own Phase D
> builds land.** Repo owner's explicit requirement: all three platforms must
> keep building and running throughout the upgrade, for end users *and* for
> anyone developing on macOS/Linux locally — not just "existing releases don't
> break." A plain workspace-wide bump doesn't satisfy that: the moment
> `agentmux-cef/Cargo.toml`'s single `cef` version pin changes, `task dev` on a
> macOS/Linux machine fails immediately (the crate-major/runtime-major
> assertion in `Taskfile.yml` would reject the still-148 runtime against a
> 152-linked binary) — loudly, by design, but still broken — until that
> platform's own 152 runtime exists.
>
> **This is now known to be low-risk, not just theoretically appealing.**
> Item 3 above was "the least predictable item in this spec" before this was
> checked. It no longer is: every one of the 13 distinct `cef::Impl*` traits
> `agentmux-cef`'s Rust source actually calls (`ImplWindow`, `ImplWindowDelegate`,
> `ImplBrowser`, `ImplBrowserHost`, `ImplFrame`, `ImplView`, `ImplViewDelegate`,
> `ImplPanel`, `ImplPanelDelegate`, `ImplTask`, `ImplAuthCallback`,
> `ImplEndTracingCallback`, `ImplOverlayController`) maps to a C++ header under
> upstream `include/`, and **all 12 of those headers are byte-for-byte identical
> between CEF `7778` and `7977`** — verified by full-text diff against the real
> files at both tags (`include/cef_auth_callback.h`, `include/cef_browser.h`,
> `include/cef_trace.h`, `include/cef_frame.h`, `include/cef_task.h`,
> `include/views/cef_{overlay_controller,panel,panel_delegate,view,
> view_delegate,window,window_delegate}.h`), not sampled or inferred from a
> changelog. (An earlier attempt at this check used GitHub's
> Contents API against `include/capi/...` paths that don't exist in this repo's
> layout — CEF's C API headers are generated at build time, not checked in —
> and silently returned empty on both sides, producing a false "no diff"
> result. The numbers above are from `raw.githubusercontent.com` fetches with
> real, non-zero byte lengths confirmed on both sides first.)
>
> **The 12 trait headers being identical isn't quite the whole surface — all
> of them transitively include `include/internal/cef_types.h`, which DOES
> drift** (119,232 → 119,729 bytes). Diffed directly: the entire delta is three
> `CEF_API_ADDED(...)`-guarded enum insertions — `CEF_CPAIT_*` (content-setting
> page-action icon type, three new members added across API versions
> 14900/15000/15200), `CEF_CTBT_TAB_SEARCH_DEPRECATED` (chrome toolbar button
> type, 15100), and `CEF_PERMISSION_TYPE_LOCAL_NETWORK_ACCESS_DEPRECATED`
> (permission-request type, 15000, converting an `#if`/`#endif` into an
> `#if`/`#elif`). These shift enum numbering for those three families between
> 148 and 152. Confirmed zero references to any of the three
> (`CPAIT`/`content_setting`, `PermissionType`/`PERMISSION_TYPE`, `CTBT`/
> `ChromeToolbarButtonType`) in `agentmux-cef/src` today — the two superficial
> `PERMISSION_TYPE` grep hits (`browser_panes/media_grants.rs`,
> `client/handlers.rs`) are the unrelated, ABI-stable
> `cef_media_access_permission_types_t` bitmask, not the drifted
> `cef_permission_request_types_t`. So "zero cfg-gating needed" still holds —
> **but the bound is on the current call surface, not a permanent guarantee.**
> If anyone later adds permission-prompt handling or content-settings UI while
> Windows is on 152 and macOS/Linux are on 148, these three enum families are
> exactly where a target split would first need `#[cfg(...)]` branching.
> (Both the transitive-header gap and its resolution: Agent5, independently,
> 2026-09-08.)
>
> **What this changes mechanically, replacing steps 1-2 above:**
> `[patch.crates-io]` is a single global override keyed by crate name — Cargo
> has no way to hold two different pinned revs of `cef-dll-sys` active for
> different targets through that mechanism. Instead:
> 1. Remove `cef` from `agentmux-cef/Cargo.toml`'s plain `[dependencies]` and
>    remove the root `[patch.crates-io]` block entirely.
> 2. Add `[target.'cfg(target_os = "windows")'.dependencies]` with
>    `cef = "152"`, patched via a target-scoped git dependency on
>    `AgentU-asaf/cef-rs@agentmux/152-begin-window-drag` (once published, per
>    step 2 above — still needed, `begin_window_drag` is confirmed absent from
>    the public 152 crate).
> 3. Add `[target.'cfg(any(target_os = "macos", target_os = "linux"))'.dependencies]`
>    with `cef = "148"`, pointed at the existing, already-working
>    `AgentU-asaf/cef-rs@515b3ac53c` rev — unchanged from today.
> 4. Validate with `cargo check --target x86_64-pc-windows-msvc` and
>    `--target x86_64-unknown-linux-gnu` (or on real macOS/Linux boxes) before
>    merging — target-specific same-crate-different-version resolution is a
>    supported Cargo pattern, but this workspace has never used it for `cef`
>    specifically and it should be proven, not assumed.
> 5. Runtime pinning needs no change beyond what Phase E already does per
>    platform (`release.yml`'s `cef-runtime-pins` job, #3085/#3086/#3089):
>    Windows's pin advances to a new 152 tag once Phase D cuts it; macOS/Linux
>    pins stay exactly as they are.
>
> **Exit condition for this staged state:** once Phase D lands working 152
> builds for macOS and Linux too, collapse the target-gated tables back into a
> single `[dependencies]` entry and delete the two `[target...]` blocks — this
> is meant to be temporary scaffolding for the rollout window, not a permanent
> architecture.
>
> **Ownership split, agreed 2026-09-08:** Agent5 owns step 2 above — rebasing
> the `begin_window_drag` slot onto the 152 binding and publishing
> `AgentU-asaf/cef-rs@agentmux/152-begin-window-drag` (`begin_window_drag`
> confirmed still absent from upstream `cef-rs` at 152, so this stays a real
> fork). AgentX owns the Cargo.toml target-gating restructuring (steps 1, 3-4)
> once that rev exists.

### Phase D — Per-platform builds (three separate machines, unavoidably)

Each platform is a distinct OS/toolchain and cannot be cross-built with the current setup. Documented cost, per platform, from `docs/cef-build/*`:

| Resource | Requirement |
|---|---|
| Wall-clock (first build, cold ccache) | **3–6 hours** |
| Disk | ~99 GB working tree + build output; **≥120 GB free** required |
| RAM | ≥32 GB (peak ~25 GB at `-j 12 -l 16`) |
| Rebuild after a patch tweak (warm ccache) | 5–30 min |

| Platform | Build doc | GN args | Output |
|---|---|---|---|
| Windows x86_64 | `docs/cef-build/build-patched-cef-windows.md` | `scripts/cef-build/args-windows.gn` | `libcef.dll` + runtime DLLs/paks |
| macOS arm64 | `docs/cef-build/build-patched-framework-macos.md` | `args-darwin.gn` | `Chromium Embedded Framework.framework` (~547 MB unstripped) |
| Linux x86_64 | `docs/cef-build/build-patched-libcef.md` | `args.gn` | `libcef.so` (strip before publishing) |

**Total build cost: 9–18 machine-hours across three machines**, plus the ~100 GB checkout on each. The three are fully independent and should run in parallel on three machines — see §6.

### Phase E — Distribution
Cut a GitHub Release per platform on `agentmuxai/cef` (manual `gh release create`; this is by documented convention a deliberate step separate from any source PR), then update consumers in *this* repo:
- `scripts/cef-build/fetch-patched-cef-windows.sh` (tag/version)
- `Taskfile.yml`'s CEF resolution tiers (`cefBuildDefault`, version-match assertion — it currently asserts `libcef.dll` major matches the linked `cef` crate major)
- `.github/workflows/build-{linux,macos,windows}.yml` — the `cef-runtime-tag` input default
- `docs/cef-build/*` version references

**Changing those input defaults is not sufficient, and assuming otherwise would silently un-pin releases.** `.github/workflows/release.yml` passes `cef-runtime-tag: ""` explicitly to all three build jobs (lines 92, 111, 150), and every callee documents blank as "latest". A caller-supplied empty string overrides the callee default, so **release builds today do not pin CEF at all — they float to whatever the newest `agentmuxai/cef` release happens to be.** Phase E must therefore set an explicit tag in `release.yml`'s three call sites, not just in the callees' defaults.

This is worth treating as its own finding, not merely a step: it means the CEF version in any given release artifact is determined by release *timing* rather than by anything in the commit, so two builds of the same commit can ship different Chromium versions. Pinning it explicitly is a prerequisite for the rollback story in Phase G being real.

> **✅ FIXED 2026-09-08 — this finding no longer describes current behaviour.**
> The two paragraphs above are kept for the reasoning; the defect itself is
> closed, by three PRs in this repo:
>
> - **#3086** pinned all three `cef-runtime-tag` call sites to literal tags,
>   replacing the blank-means-latest behaviour.
> - **#3085** centralized those three pins into a single `cef-runtime-pins`
>   job (so a bump can't update one platform and miss the others) and added a
>   Chromium-**milestone** cross-check that fails the release if they diverge.
>   The macOS half of that check is gated on `check-macos.outputs.available`,
>   so a dormant macOS pin can't block a Windows/Linux-only release.
> - **#3089** added a `workflow_dispatch` guard: an emergency rebuild must be
>   dispatched *from the tag being rebuilt*, since a run's own workflow
>   definition (and therefore its CEF pins) comes from the dispatch ref, not
>   from `inputs.tag`.
>
> **What Phase E must do now is different:** update the three literals in
> `release.yml`'s `cef-runtime-pins` job (one place, not three call sites),
> and keep all three on the same Chromium milestone or the new gate will fail
> the release by design. **Do not blank them back out.** Phase G's rollback
> story is now real, as this finding required.

**Fix the tag scheme while doing this (see §7.1).**

### Phase F — Verification matrix

Per platform, before its release tag is considered good:

| Check | Why |
|---|---|
| App boots; `chrome://gpu` shows GPU enabled, correct OS version | Guards the #1354 class of bug (unmanifested exe → Windows version-lies → GPU init CHECK-crash) |
| Native window drag works (or documented no-op fallback fires) | Patch #2 landed and ABI matches |
| Title-bar right-click menu | Patch #3 |
| Window opacity/transparency | Patch #4 |
| H.264 + HEVC playback in a browser pane | Codec GN flags survived |
| Renderer doesn't hit -67030 | Patch #1 |
| No DCHECK crash on drag/close | `dcheck_always_on=false` was actually set |
| `task package` version-integrity assertion passes | Crate major ↔ binary major agreement |

### Phase G — Rollback
Release tags are immutable, so rollback is: revert the consumer-side PR (Taskfile / workflow tags / Cargo pins) back to the `148.0.7778.180` tags. Keep the 148 releases in place permanently — do not delete them.

**This only works once Phase E's explicit pinning lands.** Until `release.yml` stops passing `cef-runtime-tag: ""`, reverting a consumer PR would *not* restore CEF 148 — a release build would still resolve "latest" and pick up the new runtime. Treat explicit pinning as the gate that makes this phase meaningful rather than as an incidental cleanup.

---

## 5. What this does *not* cover

- **Patch-level drift within 148** (`.180 → .218`). If the 152 upgrade is deferred, that smaller rebase is worth doing on its own for the security fixes, at the cost of three rebuilds and no porting. Track separately.
- Any Chromium-behavior-driven frontend changes surfaced by four milestones of rendering/DevTools/API change. Unknowable before Phase A; assume non-zero.

---

## 6. Ownership and parallelization

Phase D is the bottleneck and is embarrassingly parallel — three platforms, three machines, no shared state. Suggested split, to be confirmed by whoever picks this up:

| Phase | Can parallelize? | Notes |
|---|---|---|
| A (recon) | No — single owner | Gates everything; smallest and highest-value |
| B (fork source) | No — single owner | One branch lineage; parallel edits would conflict |
| C (Rust binding) | No | Depends on B |
| **D (builds)** | **Yes — 3 agents, one per platform** | Each needs a machine with ≥120 GB free and ≥32 GB RAM |
| E (distribution) | Partly | Per-platform release cutting parallel; consumer PR single |
| F (verification) | Yes — per platform | Same split as D |

Agents taking a Phase D platform should confirm disk/RAM headroom **before** starting — a failed build six hours in for lack of 20 GB is the expensive failure mode here.

---

## 7. Findings worth acting on regardless of whether the upgrade proceeds

### 7.1 The fork's release tags have three different version schemes
Actual tags on `agentmuxai/cef` today:

```
cef-windows-x86_64-148.0.7778.180        ← Chromium version
cef-linux-x86_64-148.0.7778.180-codecs   ← Chromium version + suffix
cef-linux-x86_64-148.0.20-3              ← CEF version + rev suffix
cef-macos-arm64-148.23.23-codecs         ← CEF framework version + suffix
cef-macos-arm64-148.0.9                  ← different scheme again
```

Three version schemes (Chromium `148.0.7778.180`, CEF `148.23.23`, CEF `148.0.9`) and ad-hoc suffixes (`-codecs`, `-2`, `-3`). A human can resolve these; **an automated drift checker cannot** — which matters directly, because the companion spec `SPEC_VERSION_DRIFT_REPORTER_2026_09_07.md` (in `a5af/shared-infrastructure`, under its own `docs/specs/`) proposes machine-comparing our shipped CEF against upstream. Recommend standardizing new tags on `cef-<platform>-<arch>-<chromium-version>[-rN]` and normalizing at read time for the historical ones.

### 7.2 A stale comment in `agentmux-cef/Cargo.toml` — ✅ FIXED 2026-09-08
The `patched-libcef` feature comment cites `https://github.com/a5af/cef, branch agentmux/7680-…`. The org redirects (`a5af` → `agentmuxai`) and that branch does still exist, so nothing breaks — but it names a CEF 146-era branch while root `Cargo.toml` documents `7778` as current. Worth correcting to whatever milestone this upgrade lands on.

> **Fixed:** now cites both relevant branches and distinguishes them, because
> they are not interchangeable: `agentmux/7778-drag-rightclick-and-transparency`
> (where the patch was authored AND where the shipped binaries were actually
> built from — macOS `148.23.23-codecs` at `6c570e249`) versus `7778` (the
> milestone integration branch it was merged into via that repo's PR #3). As of
> 2026-09-08 those two have **diverged** — the build commit is 4 ahead / 2
> behind `7778` — so the comment now says to prefer the recorded build commit
> over either branch tip when reproducing a build. Also notes the `a5af` →
> `agentmuxai` org redirect. Re-point if 152 lands on a new branch.

### 7.3 A superseded root-cause doc — ✅ FIXED 2026-09-08
`docs/analysis/archive/ANALYSIS_WINDOWS_GPU_DISABLED_ROOTCAUSE_2026_06_11.md` attributes a Windows GPU failure to the fork's libcef being a non-official/DCHECK build. Its own tracking issue (#1345) retracts that: the real cause was a missing `supportedOS` manifest (#1354, merged 2026-06-11). The archived doc still reads as if the fork were at fault, which could mislead someone scoping this upgrade. Worth a status-correction header.

> **Fixed:** the doc now opens with a RETRACTED banner stating the real cause
> (missing `supportedOS` manifest, #1354) and explicitly warning against citing
> it as evidence that the fork's build config causes GPU failures. Verified the
> retraction against #1345's own comments before writing it.

---

## 8. Open questions for the repo owner

1. **Is native window drag still a requirement on Linux specifically?** This is a Linux-only question — Windows drags via its own Win32 path and macOS via AppKit, so neither is affected by the answer (§2, patch #2). If Linux native drag were dropped, patch #2 stops needing forward-porting and this upgrade gets cheaper. Note the honest trade: the `patched-libcef` feature is already default-off, but dropping the patch means Linux window drag is a **no-op**, not a graceful fallback (§3) — so this is a real product decision, not a free saving.
2. **Is 152 the right target, or should we sit on 150/151** to stay one milestone behind the bleeding edge?
3. **Who owns the three build machines**, and do all three currently have ≥120 GB free?
4. **Should the patch-level 148 rebase (`.180 → .218`) happen now**, independent of the milestone jump, for the security fixes?
