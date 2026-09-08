# CEF 148 → 152 upgrade: Phase A reconnaissance

**Author:** Agent5
**Date:** 2026-09-08
**Status:** active — Phase A's recon is delivered by this document and its one
source-side fix shipped (`agentmuxai/cef` PR #7, merged 2026-09-08), but **two
of the spec's three Phase A checks are not fully closed**: patches #3/#4 have
not been test-applied against real 152 source (§2.4), and the macOS Xcode
toolchain pin is unconfirmed (§4). Per Codex review on PR #3093, Phase A stays
`active` until those close — the "go" in §6 is therefore conditional, not a
clearance to start Phase B blind. This is the Phase A output that
`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` §4 gates on.
**Verdict:** **Conditional go** on §3's "straight to 152" decision — see §6 for
the two conditions. Also corrects two errors in the spec's patch inventory, and
records one shipped-binary gap (§5).

---

## 0. TL;DR

| Phase A question | Answer |
|---|---|
| Do the four carried patches still apply / are any now upstream? | Inventory was **wrong in two places** — see §2. `BeginWindowDrag` is still not upstream at 152. |
| Does `cef-rs`/`cef-dll-sys` have a 152-compatible crate? | **Yes** — `cef 152.0.0+152.0.5`, published 2026-09-07. |
| Is `begin_window_drag` in its generated bindings? | **No** — same as 148, so the binding fork is still required. |
| Have the build toolchain requirements moved? | **Windows: no. Linux: no.** macOS: unconfirmed, see §4. |
| Go / no-go on targeting 152 directly? | **Conditional go** — nothing found makes 152 harder than assumed, but two Phase A checks remain open (§6). |

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

So patch #2 still needs forward-porting for 152, and the corresponding
`cef-dll-sys` binding patch is still required. **Nothing was deleted by
upstream here** — the spec's hope that four milestones might have upstreamed
some of this did not materialize for this patch.

### 2.4 Patches #2/#3/#4 status

These live on `7778` (merged via that repo's PR #3, 2026-06-25):
`views_caption_rightclick_passthrough` (registered in `patch.cfg`) and the
transparency work (`rwhv_background_opaque_check.patch` plus renderer-side
changes in `libcef/`).

**Not individually re-diffed against 152's source.** Doing that properly needs
a real 152 checkout to `git apply --check` against; asserting "applies cleanly"
from file inspection alone would be a guess. This is the one Phase A item that
is genuinely incomplete, and it is the item most likely to change Phase B's
size. Whoever takes Phase B should do this first, with a checkout.

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

## 6. Go / no-go

**Conditional go**, targeting 152 directly, per the spec's §3 reasoning.
Nothing in this recon makes 152 harder than assumed:

- the binding exists and is current;
- Windows/Linux toolchains are unchanged;
- the patch set did not grow.

It did not get materially *cheaper* either — the hope in §2 that upstream might
have absorbed some patches did not pan out for `BeginWindowDrag`, the one with
real porting cost.

**Two conditions on that "go":**

1. **§2.4 is unfinished.** Patches #2/#3/#4 have not been test-applied against
   real 152 source. Phase B should start there, and Phase B's estimate is not
   trustworthy until it does. (Patch #1 *is* now verified — §2.5.)
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
