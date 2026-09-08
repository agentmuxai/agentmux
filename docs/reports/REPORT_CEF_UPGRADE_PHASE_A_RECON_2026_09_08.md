# CEF 148 → 152 upgrade: Phase A reconnaissance

**Author:** Agent5
**Date:** 2026-09-08
**Status:** implemented — Phase A recon executed and delivered by this document;
the one source-side fix it produced shipped as `agentmuxai/cef` PR #7 (merged
2026-09-08). This is the Phase A output that
`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` §4 gates the
rest of the upgrade on. One item is deliberately incomplete — see §2.4.
**Verdict:** **Go** on §3's "straight to 152" decision — with one correction to
the spec's cost model, and one finding that is more urgent than the upgrade
itself.

---

## 0. TL;DR

| Phase A question | Answer |
|---|---|
| Do the four carried patches still apply / are any now upstream? | Inventory was **wrong in two places** — see §2. `BeginWindowDrag` is still not upstream at 152. |
| Does `cef-rs`/`cef-dll-sys` have a 152-compatible crate? | **Yes** — `cef 152.0.0+152.0.5`, published 2026-09-07. |
| Is `begin_window_drag` in its generated bindings? | **No** — same as 148, so the binding fork is still required. |
| Have the build toolchain requirements moved? | **Windows: no. Linux: no.** macOS: unconfirmed, see §4. |
| Go / no-go on targeting 152 directly? | **Go.** Nothing found makes 152 harder than the spec assumed. |

**The finding that matters most is not about 152 at all:** none of the four
patches were in any shipped CEF binary (§5). One of them is a macOS renderer
crash fix. That is now partially addressed (`agentmuxai/cef` PR #7, merged)
but still needs builds.

---

## 1. Method, and its limits

Everything below was verified against the actual repositories via the GitHub
API — not read off the spec, which turned out to contain errors this pass
corrected. Where a claim could not be verified from here, it says so rather
than guessing.

**Hard limit:** no Chromium checkout was available (~100 GB), and no
macOS/Linux build machine. So "does this patch still apply to 152's source"
was answered by inspecting upstream 152's actual source files for the code
each patch targets — not by running `git apply` against a real 152 tree. That
distinction matters and is called out per-patch below.

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

## 5. The finding that outranks the upgrade

While tracing patches, all three currently-pinned CEF release tags were
resolved to their actual commits:

| Pinned tag | Commit | Cut |
|---|---|---|
| `cef-windows-x86_64-148.0.7778.180` | `05d7a247` | 2026-04-23 |
| `cef-linux-x86_64-148.0.7778.180-codecs` | `05d7a247` | 2026-04-23 |
| `cef-macos-arm64-148.23.23-codecs` | `05d7a247` | 2026-04-23 |

All three are **the same commit**, cut **2026-04-23** — which predates the
process-requirement patch (2026-06-02) and the drag/right-click/transparency
merge (2026-06-25). Only one later release exists at all
(`cef-macos-arm64-148.23.21`, 2026-07-02, built from the
drag/rightclick/transparency branch), and nothing in this repo references it —
note also that its version label (`148.23.21`) is *lower* than the tag we pin
(`148.23.23`) despite being newer, which is §7.1's tag-scheme problem biting in
practice.

**So none of the four documented fork patches are in any CEF binary AgentMux
ships today.**

The one that matters: per `docs/retro/retro-macos26-cef-dcheck-root-cause-2026-06-02.md`,
the -67030 Mach-port peer validation failure is "a *real* functional code-sign
failure... not a DCHECK" and was "the only reason we went from-source [CEF] at
all" — without it, the self-reexec CEF helper fails peer validation and the
renderer crash-loops on macOS 26.

**Status:** source side is now fixed (PR #7 merged into `7778`, patch
registered, apply-failure fixed). **Not shipped** — that needs an arm64 macOS
build (Xcode, ≥120 GB, ≥32 GB RAM, 3–6 h) and a fresh release cut. Windows and
Linux should be re-cut from the same tip at the same time so all three land on
one source commit again.

This is worth doing **independent of the 152 upgrade**, and is far cheaper:
no porting, no API fallout — merge (done), build, release, repoint pins.

---

## 6. Go / no-go

**Go**, targeting 152 directly, per the spec's §3 reasoning. Nothing in this
recon makes 152 harder than assumed:

- the binding exists and is current;
- Windows/Linux toolchains are unchanged;
- the patch set did not grow.

It did not get materially *cheaper* either — the hope in §2 that upstream might
have absorbed some patches did not pan out for `BeginWindowDrag`, the one with
real porting cost.

**Two conditions on that "go":**

1. **§2.4 is unfinished.** Patches #2/#3/#4 have not been test-applied against
   real 152 source. Phase B should start there, and Phase B's estimate is not
   trustworthy until it does.
2. **Do §5 first, or at least in parallel.** Shipping a macOS renderer crash
   fix that has been written since June should not wait behind a
   four-milestone Chromium jump.

---

## 7. Open items not addressed here

- **§7.1 (tag schemes)** — untouched. Release tags are immutable, so the
  historical inconsistency can only be normalized at read time, not fixed in
  place. Worth doing before any automated drift checker is built against these
  tags; §5's `148.23.21`-newer-than-`148.23.23` case shows it already misleads.
- **§8 open questions** (Linux native drag still required? 150/151 vs 152?
  who owns the build machines? do the `.180 → .218` patch-level rebase now?)
  are repo-owner decisions and remain open — recon does not answer them, though
  §5 makes question 4 more pressing, since a rebuild is needed regardless now.
