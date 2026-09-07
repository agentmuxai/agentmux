# CEF milestone upgrade: 148 (7778) → 152 (7977), all three platforms

**Author:** AgentX
**Created:** 2026-09-07
**Status:** proposed — analysis complete, not yet implemented
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
| 1 | `agentmux_process_requirement.patch` (`GetPeerValidationPolicy() → kNoValidation`, the -67030 renderer fix) | Not upstream | All platforms | **Yes** — mandatory, registered in `patch/patch.cfg` |
| 2 | `CefWindow::BeginWindowDrag()` (native HTCLIENT-region window drag) | Not upstream as of 148 — **re-check against 152** | **Linux only.** Windows uses its own Win32 path (`post_win32_begin_move`) and never calls this — `scripts/cef-build/args-windows.gn:4` states the patch "never [was] needed" there; macOS uses AppKit drag regions. Gated by the `patched-libcef` feature | **Yes, if Linux native drag stays in scope** — nothing else depends on it |
| 3 | HTCAPTION right-click fall-through to renderer | Not upstream as of 148 — **re-check against 152** | Title-bar right-click menus | Yes, if that UX is kept |
| 4 | Transparency cascade (RWHView/WebContents bg) | **Partial** upstream since 148 | Window opacity | Re-verify; may be droppable |

Plus **build flags, not patches** — version-controlled in *this* repo, not the fork:
`scripts/cef-build/args.gn` (Linux), `args-darwin.gn` (macOS), `args-windows.gn` (Windows). These carry `proprietary_codecs=true`, `ffmpeg_branding="Chrome"`, HEVC/AC3/EAC3/Dolby Vision enables, Widevine, and (critically) the `dcheck_always_on=false` requirement — a from-source `is_official_build=false` build defaults DCHECKs *on*, which crashes on drag/close (see `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md`).

**Re-checking #2, #3 and #4 against upstream 152 is the first task in this spec, because the answer can delete work.** Between 146 and 148, several AgentMux patches (`CefV8BackingStore`, `CefComponentUpdater`, `blink_ax_viewport_collapse`, the task-manager shutdown fix) were upstreamed and stopped being ours to carry. The same may have happened again across four milestones.

---

## 3. Decision: target 152 directly, not milestone-by-milestone

**Recommendation: go straight to 7977 (Chromium 152).** Rationale:

- The per-milestone cost is dominated by *rebuild* (3–6 h × 3 platforms), not by *porting*. Stepping through 149 → 150 → 151 → 152 multiplies the expensive part by four while only marginally easing the porting part.
- Our patch set is small (4 items, one mandatory) and touches stable CEF surface (`cef_window.h`, `window_impl.{cc,h}`, `window_view.cc`, `browser_view_impl.cc`).
- We have an ABI safety net, though a narrower one than "drag keeps working": `agentmux-cef/src/ui_tasks/drag.rs` compares the runtime `_cef_window_t.size` against the compiled binding's and **returns early with a warning** if they diverge, so an unpatched or mismatched runtime cannot read the extension slot (which would be UB). It prevents memory unsafety; it does **not** preserve the feature. On Linux the practical result is that native window drag becomes a no-op. Windows and macOS are unaffected either way — they never route through this patch (see §2, patch #2).

**Counter-case, stated honestly:** if the port hits a wall on 152, the fallback is to land on 7871 (150) — still three milestones of progress — rather than abandoning the effort. Decide this only if 152 porting exceeds ~2 days.

---

## 4. Work breakdown

### Phase A — Reconnaissance (no builds; do this before committing to the rest)
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

### 7.2 A stale comment in `agentmux-cef/Cargo.toml`
The `patched-libcef` feature comment cites `https://github.com/a5af/cef, branch agentmux/7680-…`. The org redirects (`a5af` → `agentmuxai`) and that branch does still exist, so nothing breaks — but it names a CEF 146-era branch while root `Cargo.toml` documents `7778` as current. Worth correcting to whatever milestone this upgrade lands on.

### 7.3 A superseded root-cause doc
`docs/analysis/archive/ANALYSIS_WINDOWS_GPU_DISABLED_ROOTCAUSE_2026_06_11.md` attributes a Windows GPU failure to the fork's libcef being a non-official/DCHECK build. Its own tracking issue (#1345) retracts that: the real cause was a missing `supportedOS` manifest (#1354, merged 2026-06-11). The archived doc still reads as if the fork were at fault, which could mislead someone scoping this upgrade. Worth a status-correction header.

---

## 8. Open questions for the repo owner

1. **Is native window drag still a requirement on Linux specifically?** This is a Linux-only question — Windows drags via its own Win32 path and macOS via AppKit, so neither is affected by the answer (§2, patch #2). If Linux native drag were dropped, patch #2 stops needing forward-porting and this upgrade gets cheaper. Note the honest trade: the `patched-libcef` feature is already default-off, but dropping the patch means Linux window drag is a **no-op**, not a graceful fallback (§3) — so this is a real product decision, not a free saving.
2. **Is 152 the right target, or should we sit on 150/151** to stay one milestone behind the bleeding edge?
3. **Who owns the three build machines**, and do all three currently have ≥120 GB free?
4. **Should the patch-level 148 rebase (`.180 → .218`) happen now**, independent of the milestone jump, for the security fixes?
