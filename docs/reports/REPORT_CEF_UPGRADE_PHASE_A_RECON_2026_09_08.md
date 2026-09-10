# CEF 148 → 152 upgrade: Phase A reconnaissance

**Author:** Agent5
**Date:** 2026-09-08
**Status:** active — Phase A's recon is delivered by this document and its one
source-side fix shipped (`agentmuxai/cef` PR #7, merged 2026-09-08), **Patches #1/#2/#3 are verified against real 152
source** (§2.4/§2.5). **Patch #4's five-commit CEF-side cascade is now ported**
(§5b) — its Chromium-side file applies cleanly and the cascade merged cleanly
onto 7977. **Windows is now compile-verified** (below); macOS/Linux are not.
Also deferred: the macOS
hermetic Xcode pin (§4), a Phase D build-time check. **Phase B's port is
COMPLETE — 18 of 18 files** (§5b), and **compiled successfully on Windows**
(`libcef.dll` built and boot-verified 2026-09-09, see the toolchain correction
below) — macOS and Linux remain unbuilt. This is the Phase A output that
`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` §4 gates on.
**Verdict:** **Go** on §3's "straight to 152" decision — the *milestone target*
is sound: nothing found makes 152 harder, and 16 of the 18 fork-modified CEF
files are byte-identical upstream. **This is not a statement that the port is
compile-verified on all platforms** — Phase B is 18/18 ported (§5b) and
**Windows has actually been built and boots** (2026-09-09); macOS and Linux
have not (§2.4, §5b). Also corrects two errors in the spec's patch inventory, and records
one shipped-binary gap (§5).

---

## 0. TL;DR

| Phase A question | Answer |
|---|---|
| Do the four carried patches still apply / are any now upstream? | Inventory was **wrong in two places** — see §2. `BeginWindowDrag` is still not upstream at 152. |
| Does `cef-rs`/`cef-dll-sys` have a 152-compatible crate? | **Yes** — `cef 152.0.0+152.0.5`, published 2026-09-07. |
| Is `begin_window_drag` in its generated bindings? | **No** — same as 148, so the binding fork is still required. |
| Have the build toolchain requirements moved? | **Corrected 2026-09-09** — Windows: *pins* say no, but a real 152 build hit a hard blocker the pins don't show (§4). Linux: no (unverified by an actual build). macOS: unconfirmed. |
| Go / no-go on targeting 152 directly? | **Go.** #1/#2/#3 verified vs real 152 source; #4's Chromium-side file verified and its CEF-side cascade ported (§5b). 16 of 18 CEF files byte-identical upstream; the 2 drifted merged cleanly. **Phase B 18/18 ported. Windows built and boot-verified 2026-09-09; macOS/Linux not built.** macOS Xcode pin deferred to Phase D. |

**A separate finding, not about 152:** patch #1 (the macOS -67030 renderer
crash fix) was never registered in `patch/patch.cfg`, so no build on any
platform could ever have applied it — it is absent from the shipped macOS
binary. Now fixed at the source (`agentmux/cef` PR #7) but **not yet built**;
needs one arm64 macOS rebuild. See §5, including a correction: an earlier draft
of this report over-claimed that *no* fork patches ship, which was wrong — the
other three do.

---

## 1. Method, and its limits

Everything below was verified against the actual repositories via the GitHub
API — not read off the spec, which turned out to contain errors this pass
corrected. Where a claim could not be verified from here, it says so rather
than guessing.

**Hard limit:** no full Chromium checkout was available (~100 GB), and no
macOS/Linux build machine.

For **patch #1** this limit was worked around and the check is real: its target
file is a single source file, so it was fetched at the exact Chromium 152 tag
and the patch applied to it with CEF's own patcher config
(`git apply -p0 --ignore-whitespace`) — see §2.5. For **patches #2/#3/#4**, which
touch many files across `libcef/`, the limit stands: they were assessed by
inspecting upstream 152's sources, **not** by a real apply. That distinction is
called out per-patch, and §6 makes it a condition on the "go."

---

## 2. Corrected patch inventory

The spec's §2 table has two errors.

### 2.1 Patch #1 is macOS-only, not "All platforms"

The spec lists `agentmux_process_requirement.patch` as `Used by: All platforms`.
It patches `base/apple/mach_port_rendezvous_mac.cc`. Mach ports are a Darwin
kernel primitive and Chromium's `base/apple/` tree is not compiled on Windows
or Linux — this patch **cannot affect** those platforms. It is macOS-only.

Consequence: it is not a blocker for a Windows/Linux-only 152 build, and the
"mandatory, all platforms" framing overstates its reach.

### 2.2 Patch #1 was not registered, and not merged

The spec says it is "registered in `patch/patch.cfg`." It was not — neither the
`patch.cfg` entry nor the patch file existed on `7778`. It sat on an unmerged,
diverged branch (`agentmux/7778-process-requirement`, 1 ahead / 13 behind)
from 2026-06-02 until it was merged as part of this recon
(`agentmuxai/cef` PR #7, 2026-09-08).

That PR also fixed a real defect in the patch itself, found by Codex review:
its diff header used `a/`/`b/` prefixes, but CEF's patcher
(`tools/git_util.py`) runs `git apply -p0`, which does no path stripping — so
it looked for a literal `a/base/apple/...` and failed. Since
`cef_create_projects.sh` exits on a failed patch, **this would have stopped
every source build touching that branch**, on any platform. Verified by
reconstructing the target file and running CEF's exact patcher config against
both versions.

### 2.3 `BeginWindowDrag` is still not upstream at 152

Checked `include/views/cef_window.h` at upstream branch `7977` directly: it has
`SetDraggableRegions()` and no `BeginWindowDrag()`. Our fork's `7778` does have
it (`/*--cef(added=14800)--*/ virtual bool BeginWindowDrag() = 0;`).

So patch #2 must still be **carried** for 152, and the corresponding
`cef-dll-sys` binding patch is still required. **Nothing was deleted by
upstream here** — the spec's hope that four milestones might have upstreamed
some of this did not materialize. Note that "carried" turned out to be much
cheaper than "forward-ported": see §2.4 — upstream never touched the files it
edits, so moving it is a copy.

### 2.4 Patches #2/#3/#4 status — ✅ NOW VERIFIED against real 152 source

An earlier revision left this open, assuming a ~100 GB Chromium checkout was
needed. For the **Chromium-side** files it wasn't: those were fetched at tag
`152.0.7977.83` and the patch applied with CEF's own patcher config
(`git apply -p0 --ignore-whitespace`). Patch #2 needed a different method
because it edits CEF's *own* sources rather than Chromium's.

**Read this table narrowly.** It reports per-*file* apply results, not
per-*patch* completeness. Patch #3 is fully covered by its one Chromium file.
**Patch #4 is not** — it also carries a five-commit CEF-side transparency
cascade that this table does **not** cover. That cascade **has since compiled
successfully** as part of the Windows build (2026-09-09, §5b) — but compiling
is not the same claim as "the transparency effect works correctly at
runtime," which was never tested (the boot check only confirmed a plain page
navigates, not a transparent-window use case). See the caveat below and §5b.
Patch #1 is in §2.5.

| Patch | Target(s) | Method | Result |
|---|---|---|---|
| #3 `views_caption_rightclick_passthrough` | `ui/views/widget/desktop_aura/window_event_filter_linux.cc` | real `git apply -p0` vs Chromium 152 | **applies cleanly** |
| #4 — ⚠️ `rwhv_background_opaque_check` **only** (this is *one part of* patch #4, **not** patch #4) | `content/browser/renderer_host/render_widget_host_view_base.cc` | real `git apply -p0` vs Chromium 152 | **that file applies cleanly.** The rest of patch #4 (CEF-side cascade) has since **compiled** on Windows (§5b) but its runtime behavior is untested — treat as "compiles," not "works." |
| #2 `BeginWindowDrag` | `include/views/cef_window.h`, `libcef/browser/views/window_impl.{cc,h}` | `cmp` upstream 7778 vs 7977 | **all three byte-identical** → port is mechanical |

Patch #2's result is the notable one: **upstream CEF did not touch any of its
three target files across four milestones** (confirmed with `cmp`, not a
line-diff heuristic). So the fork's 7778 copies equal upstream-7977 plus our
changes exactly, and the "port" is a copy rather than a forward-port. Our delta
there is ~103 changed lines across the three files.

Note also that the spec lists four files for patch #2; `libcef/browser/views/window_view.cc`
contains no `BeginWindowDrag` reference on either side and is **not** part of it.

> **Caveat on patch #4 (Codex, PR #3095).** The row above covers only patch #4's
> single *Chromium-side* file. Patch #4's transparency cascade is five coupled
> **CEF-side** commits as well; those are part of the 18-file surface in §5b
> and have since **compiled successfully on Windows** (§5b, 2026-09-09) — not
> yet on macOS/Linux, and never functionally exercised at runtime on any
> platform. Treat #1/#2/#3 as apply-verified, #4's cascade as
> Windows-compile-verified but functionally untested.

**Consequence: most of the patch surface costs little to move to 152** — 16 of
18 fork-modified CEF files are byte-identical upstream, so they copy rather than
port. But **2 genuinely drifted** and need real merge + compile work (§5b), and
the rebuilds remain the dominant cost as §3 of the spec predicted.

### 2.5 Patch #1 verified to apply cleanly to Chromium 152

Done as the first piece of Phase B, and it closes the patch-#1 half of Phase A
task 1 properly rather than by inspection:

- Fetched `base/apple/mach_port_rendezvous_mac.cc` at Chromium tag
  `152.0.7977.83`. The target function `GetPeerValidationPolicy()` is
  **byte-identical** to the 148 version the patch was written against — only
  its line number moved (405 → 407), which `git apply` resolves by context.
- Applied the patch (as merged on `7778`) with CEF's exact patcher config:
  **applies cleanly**, producing the correct `kNoValidation` return.

So patch #1 forward-ports to 152 with **zero modification**. Landed on
`agentmux/7977-process-requirement` in the fork, registered in 152's
`patch.cfg` (whose tail differs from 148's — it ends with
`chrome_browser_extensions_background`, so the insertion point is not the same).

---

## 3. Rust binding (`cef-rs` / `cef-dll-sys`)

**A 152-compatible crate exists.** `tauri-apps/cef-rs` published
`cef 152.0.0+152.0.5` to crates.io on **2026-09-07** — one day before this
recon, and the same day the upgrade spec was written. Version history is
regular (149 → 150 → 151 → 152 over ~10 weeks), so the binding project is
keeping pace with CEF stable branches.

**`begin_window_drag` is not in it.** It is absent from upstream `cef-rs`
entirely, which follows directly from §2.3 — the binding is generated from
CEF's own C API, and CEF 152 has no such method. Our fork
(`AgentU-asaf/cef-rs`, branch `agentmux/148-begin-window-drag`) is the only
place it exists, and Phase C's plan to rebase that onto the 152 binding stands
unchanged.

---

## 4. Build toolchain requirements

Compared `chromium/chromium` at the two exact tags each CEF branch pins
(`148.0.7778.218` and `152.0.7977.83`, both read from
`CHROMIUM_BUILD_COMPATIBILITY.txt` — these confirm the spec's version table):

| Platform | Signal checked | Result |
|---|---|---|
| Windows | `build/vs_toolchain.py` → `TOOLCHAIN_HASH` | **Unchanged** (`e66617bc68` at both tags) |
| Windows | `MSVS_VERSIONS` | **Unchanged** — VS 2026 (18.0) packaged toolchain at both |
| Linux | `build/linux/sysroot_scripts/sysroots.json` | **Unchanged** — same bullseye sysroot set |
| macOS | hermetic Xcode pin | **Not confirmed** — could not locate the version pin through the files checked |

So Windows and Linux need no toolchain change for this upgrade. **macOS is
unverified**, not verified-as-fine — confirm it at Phase D build time on the
actual machine rather than trusting this table for that row.

> **Correction (2026-09-09, from an actual Windows 152 build on claudius).**
> "Windows: unchanged" above is true about the *pins* and was **wrong about
> buildability**. A real build failed at 22,219/59,112 targets:
> `ui/accessibility/platform/uia_client_info_source_win.cc` (new in Chromium
> 152, unconditional in the Windows `BUILD.gn`) needs
> `IUIAutomationClientInfo{,Source}` interfaces **absent from Windows SDK
> 10.0.26100.0** — the exact SDK `vs_toolchain.py`'s `TOOLCHAIN_HASH`/
> `SDK_VERSION` checks above confirmed unchanged, and the only SDK VS Build
> Tools 2022's installer catalog offers. Google's internal
> `DEPOT_TOOLS_WIN_TOOLCHAIN=1` toolchain bundles a different snapshot of
> "26100" than the public installer, so this only bites external builders.
> **Fixed** by installing a newer public SDK (`10.0.28000.0`) and editing
> `SDK_VERSION` in *both* `build/vs_toolchain.py` and
> `build/toolchain/win/setup_toolchain.py` — the GN arg alone doesn't
> propagate. Full detail: `docs/cef-build/build-patched-cef-windows.md`'s
> 2026-09-09 update. **Lesson repeated a third time in this same recon
> effort** (after the API-annotation bug and the invalid `strings`-based
> symbol check): comparing version pins is not a substitute for actually
> compiling. Treat every "unchanged" toolchain claim in this table as
> "unchanged in the numbers checked," not "confirmed buildable," until an
> actual build says otherwise — which is now true for Windows but still not
> for Linux.

---

## 5. The shipped-binary gap — corrected

> **Correction (2026-09-08, before merge).** An earlier draft of this section
> claimed *"none of the four documented fork patches are in any CEF binary
> AgentMux ships today,"* inferred from all three release tags' `target_commitish`
> resolving to one April-23 commit. **That inference was invalid and the claim
> was false.** A GitHub release's tag target is not build provenance — a release
> cut without `--target` inherits the default branch's HEAD. Caught in review by
> Codex on PR #3093. What follows is the corrected, directly-verified version.
> The scope of the real problem is much narrower than the retracted claim.

### 5.1 What actually ships (verified, not inferred)

The authoritative provenance is the build records, not the tags:

- **macOS** — `STATUS_CEF_PROPRIETARY_CODECS_MACOS_2026_07_27.md` records
  `cef-macos-arm64-148.23.23-codecs` as built 2026-07-28 from fork commit
  `6c570e249`, with **112 patches applied, 3 skipped, 0 failed**, and
  `BeginWindowDrag` explicitly re-verified in the built framework
  (`verify-cef-framework-darwin.sh`, exit 0). The framework's own
  `CEF_COMMIT_HASH` embeds `6c570e2490c9…`, which is the real provenance.
- **Windows** — `docs/cef-build/build-patched-cef-windows.md` states it is
  "built from the same fork branch anyway for one-canonical-source-branch"
  reasons, with the drag/transparency patches "present but inert on Windows."

Checked directly against that build commit (`6c570e249`):

| Patch | In the shipped binaries? |
|---|---|
| #2 `BeginWindowDrag` | **Yes** — present in `include/views/cef_window.h` |
| #3 rightclick passthrough | **Yes** — registered in `patch.cfg` |
| #4 transparency cascade | **Yes** — same branch lineage |
| #1 `process_requirement` | **No** — absent from `patch.cfg` at that commit |

So the fork's patch pipeline is working, and is demonstrably careful: that same
status doc records catching a `patcher.py --root-dir` bug that would have
silently skipped **all 112 patches**, before the long build started.

### 5.2 The real gap, narrowed

**Patch #1 alone is missing, and for a specific reason:** it was never
registered in `patch/patch.cfg` on any branch, so `patcher.py` — which reads
that file to decide what to apply — could never have applied it, on any build,
on any platform. It was not a stale-release problem; it was an
unregistered-patch problem. That part of the original finding stands, and was
verified by direct file checks (404 on the file, zero `patch.cfg` matches), not
by the tag inference that was wrong.

This still matters, because per
`docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md` the -67030
Mach-port peer validation failure is "a *real* functional code-sign failure...
not a DCHECK" and was "the only reason we went from-source [CEF] at all" —
without it, the self-reexec CEF helper fails peer validation and the renderer
crash-loops on macOS 26.

**Status:** source side fixed — `agentmuxai/cef` PR #7 merged the patch into
`7778` *and registered it in `patch.cfg`*, which is the part that was actually
missing. Also fixed a defect that would have made it fail to apply even once
registered (`a/`/`b/` prefixes vs `git apply -p0`).

**Still not shipped**, and this is now a **macOS-only** rebuild need — Windows
and Linux are not affected by patch #1 (§2.1) and already carry the patches
that do apply to them, so **the "re-cut all three platforms" recommendation in
the retracted draft was unnecessary work**. One arm64 macOS build (Xcode,
≥120 GB, ≥32 GB RAM, 3–6 h) and one release cut closes it.

### 5.3 Method note, kept deliberately

The retracted claim came from treating `target_commitish` as build provenance.
It is not, and this repo's own docs say so — `STATUS_CEF_PROPRIETARY_CODECS_MACOS_2026_07_27.md`
notes that releases cut without `--target` inherit a default-branch timestamp,
which is also why `gen-docs-index`-style ordering by release metadata is
unreliable here. **Provenance for these binaries lives in the build status docs
and in the framework's embedded `CEF_COMMIT_HASH` — check those, not the tag.**
Recorded so the next person doesn't repeat it.

---

## 5b. Phase B — patch port to 7977 (COMPLETE, 18/18 — Windows built)

> **History, in order, each correcting the last:** (1) 2026-09-08, claimed
> "all four ported" after testing one file of patch #4 — wrong, Codex caught
> it on PR #3095; corrected to 3/18 measured properly. (2) 2026-09-09,
> completed the remaining 15 files via real per-file 3-way merges (18/18,
> below). (3) 2026-09-09, built and boot-verified on Windows (§ below).
> Kept all three states visible rather than collapsing straight to the
> current one — this doc's own recurring failure mode has been updating a
> conclusion and leaving the reasoning behind it stale.

### Actual porting surface — 18/18 ported

| | Count | Meaning |
|---|---|---|
| Fork-modified CEF source files | **18** | `libcef/**` + `include/{views,internal}/**`, excluding generated `capi`/`libcef_dll` |
| Byte-identical upstream 7778 ↔ 7977 | **16** | mechanical copy — correct by construction |
| Drifted, forward-ported via real 3-way merge | **2** | `include/internal/cef_types.h`, `libcef/renderer/render_manager.cc` — merged cleanly, zero conflicts, because our changes and upstream's occupy different regions of each file |
| **Ported, total** | **18/18** | |

Most of the port really was mechanical, because upstream barely touched this
surface across four milestones — but note the two drifted files include
`render_manager.cc`, which is patch #4's *renderer-side* transparency
cascade half. Its earlier "not verified" status (§2.4) is now **compiled**,
not just ported — see the Windows build below.

### Branch layout — `7977` is now a real integration branch

| Branch | Contents |
|---|---|
| `7977` | **integration branch** (was the bare upstream mirror until 2026-09-09 — see the merge below) |
| `agentmux/7977-process-requirement` | patch #1, registered in `patch.cfg` |
| `agentmux/7977-drag-rightclick-and-transparency` | patch #2's three source files; patches #3 and #4, both registered in `patch.cfg` |

**Gap found and fixed, 2026-09-09.** Unlike `7778` — where patches are merged
directly into the milestone branch, so checking it out gets everything —
`7977` was, until this fix, still the bare upstream mirror (`0` ahead/behind
`chromiumembedded/cef`). The two feature branches above existed but had never
been merged into it. Checking out bare `7977` per this repo's own "always
build from the integration branch" convention would have silently built
**zero of the four patches** — caught while fixing a Codex finding on
PR #3130 about the build doc's 152 instructions. Fixed by merging both
feature branches into `7977` directly (one conflict — both branches append to
`patch.cfg`'s tail at the same anchor; resolved by keeping both entries).
Verified post-push via the API: all three `patch.cfg` entries present,
`BeginWindowDrag` present.

**A registration gap was also found and deliberately not carried forward.**
Patch #4 (`rwhv_background_opaque_check`) is registered on
`agentmux/7778-drag-rightclick-and-transparency` — the branch the shipped
148 binaries were actually built from — but **not** on the `7778` integration
branch. A port done "from `7778`" would have silently dropped it; it's
registered on the 152 branch instead. Same divergence as §5.1: the 148 build
branch and its integration branch are 4 ahead / 2 behind each other and do
not contain the same patch set.

### Windows: built and boot-verified, 2026-09-09

`libcef.dll` compiled from the completed `7977` integration branch —
305,693,184 bytes, zero compile/link errors (after two build-environment
fixes unrelated to the port itself: a Windows SDK version gap and an
unrelated upstream CEF test-suite bug, both in §4/the build doc). Boot-tested
via `cefsimple.exe`: real top-level window, `MainWindowTitle = "Example
Domain"` after navigating — genuine render, not just process launch.

**What this does and doesn't establish:** the port *compiles* on Windows,
including patch #4's CEF-side cascade. It does **not** establish that the
cascade's actual transparency behavior is correct at runtime — that was
never exercised (the boot test loads an opaque page). It also says nothing
about macOS or Linux, which remain unbuilt and may still hit compile issues
this pass didn't surface — Windows never routes through patches #2/#3/#4
functionally in the first place (§2.1-§2.3), so a clean Windows compile is
weaker evidence for those platforms than it looks.

---

## 6. Go / no-go

**Go**, targeting 152 directly, per the spec's §3 reasoning. Nothing in this
recon makes 152 harder than assumed, and the patch work turned out cheaper:

- the binding exists and is current;
- Windows/Linux toolchain *pins* are unchanged, though Windows needed a newer
  public SDK than those pins implied to actually build (§4 correction) —
  resolved, not a blocker, but real;
- the patch set did not grow.

It did not get materially *cheaper* either — the hope in §2 that upstream might
have absorbed some patches did not pan out for `BeginWindowDrag`, the one with
real porting cost.

**Two conditions on that "go":**

1. **Patch applicability — CLOSED.** #1/#2/#3 verified against real 152 source
   (§2.4, §2.5); #4's Chromium-side file likewise, and its CEF-side cascade is
   now ported via clean 3-way merge (§5b). All 18 files are on the 152
   branches. **The remaining unknown is compilation** — none of this has been
   built, which Phase D resolves.
2. **§4's macOS toolchain row is unconfirmed**, not confirmed-fine. Establish
   the hermetic Xcode requirement on the actual machine before committing to a
   macOS Phase D slot.
3. **Do §5 in parallel, not after.** The macOS -67030 fix has been written
   since June and still isn't in a shipped binary; it needs one macOS rebuild
   and shouldn't wait behind a four-milestone Chromium jump. Note this is
   **macOS-only** — the earlier draft's "re-cut all three platforms" was
   over-scoped and is retracted.

---

## 7. Open items not addressed here

- **§7.1 (tag schemes)** — untouched. Release tags are immutable, so the
  historical inconsistency can only be normalized at read time, not fixed in
  place. Worth doing before any automated drift checker is built against these
  tags; the `148.23.21`-newer-than-`148.23.23` case shows it already misleads.
  Related: §5.3's method note — release *tags* are not build provenance either.
- **§8 open questions** (Linux native drag still required? 150/151 vs 152?
  who owns the build machines? do the `.180 → .218` patch-level rebase now?)
  are repo-owner decisions and remain open — recon does not answer them, though
  §5 makes question 4 more pressing for macOS specifically, since that platform
  needs a rebuild regardless now.
