# CEF Fork Maintenance — Branch Model and Upgrade Practice

**Status:** living
**Applies to:** `agentmuxai/cef` (our CEF fork), all three platforms
**Companion docs:** [build-patched-libcef.md](./build-patched-libcef.md) (macOS/Linux),
[build-patched-cef-windows.md](./build-patched-cef-windows.md),
[build-patched-framework-macos.md](./build-patched-framework-macos.md)

We maintain a fork of CEF carrying five AgentMux-specific changes. Every Chromium
milestone upgrade has to carry that set forward, for Windows, Linux and macOS
together. This doc is the standing practice for doing that without losing pieces.

It exists because we lost pieces twice.

---

## 1. The two failures this prevents

Both are **silent**. Neither produced an error, a conflict, or a failed build.
That is the defining property of this class of bug: you find out when a user
reports a feature that used to work.

### 1.1 The forward-merge gap (2026-07-01 → 2026-09-09, found in production path)

`agentmux/7778-drag-rightclick-and-transparency` was merged into `fork/7778` via
PR #3 on Jun 24. Then PRs #4 and #5 added two **more** commits to that same
branch on Jul 1 — the renderer-side transparency work and the macOS
white-substitution fix. Nobody merged the branch forward a second time.

```
2720ba103  (Jun 24)  drag branch
    │
    ├──► merged into fork/7778 as 2d7833dec   (PR #3, Jun 24 18:57)
    │         │
    │         └──► 5387e4974  (PR #7, Sep 8)  ── fork/7778 tip
    │                                             has process_requirement
    │                                             MISSING #4 and #5
    │
    └──► 7ba6b59f4  (PR #4, Jul 1 19:19)      parent = 2720ba103
              │
              └──► 6c570e249 (PR #5, Jul 1 20:06)  ── drag branch tip
                                                      what actually SHIPPED
```

For two months this cost nothing, because the July macOS framework was built from
the drag branch tip directly (`cef HEAD: 6c570e249` in the build log), not from
`fork/7778`. It only bit when someone went to `fork/7778` to pick up the
`process_requirement` P0 fix — the reasonable, obvious move. Every signal pointed
that way: `fork/7778` is the integration branch, it is named for the Chromium
version, and it carried the newest commit (Sep 8 vs Jul 1).

**Newer-looking is not the same as superset.** Building from `fork/7778` would
have fixed the renderer crash-loop and simultaneously regressed macOS window
transparency, with nothing anywhere reporting a problem.

### 1.2 Always re-fetch before you judge a branch

An earlier revision of this doc claimed the Chromium 152 port had dropped the
renderer-side transparency work. **That was wrong**, and the way it was wrong is
itself worth keeping.

The probe was real, but it ran against a stale local ref.
`agentmux/7977-drag-rightclick-and-transparency` had been **force-updated**
(`d6fe3d449` -> `55edc030a`); after `git fetch --prune` all seven transparency
files probe `OK`. The 152 port is complete. Two existing documents already said
so -- `docs/reports/REPORT_CEF_UPGRADE_PHASE_A_RECON_2026_09_08.md` and
`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` -- and were not
consulted.

So, as a rule:

- **`git fetch --prune` immediately before running §5.** These branches get
  force-updated during a port. A ref fetched yesterday is not evidence about
  today.
- **Cross-check against the milestone's own recon/spec docs** before reporting a
  gap. A disagreement between your probe and a written port record means one of
  them is stale -- find out which rather than trusting the probe because you ran
  it yourself.
- A probe reporting `MISS` proves something is absent **from the ref you
  probed**. That is a weaker claim than "absent from the branch," and the
  difference is exactly the mistake made here.

One genuine asymmetry does remain, and it is the R4 case rather than a defect:
`agentmux_process_requirement` is absent from the drag branch because it lives on
`agentmux/7977-process-requirement`. Both must be merged into `fork/7977`. That
split is precisely what produced §1.1 on 7778.

---

## 2. Mental model: our changes live in two layers

This is the single most important thing to internalise. The two layers have
**completely different mechanics**, and conflating them is what makes the gaps
invisible.

| | Layer A — Chromium-side patches | Layer B — libcef-side commits |
|---|---|---|
| **What** | `.patch` files under `cef/patch/patches/` + registration in `cef/patch/patch.cfg` | Ordinary edits to `cef/libcef/**` and `cef/include/**` |
| **Applied by** | `patcher.py`, at build time, into the surrounding Chromium tree | Nothing. They are just commits on the branch. |
| **How it breaks** | Patch fails to apply, or `patcher.py` silently applies zero | Branch simply doesn't contain the commit |
| **Detection** | Loud (`.rej` files, non-zero exit) — *unless* the invocation is wrong, see §6.1 | **Silent.** Nothing checks. |

Layer B is where both failures happened. There is no tool that notices a missing
libcef commit — the tree still compiles, because our additions are additive.

A corollary that matters for the upgrade runbook: `patch.cfg` is a **single file
shared by all three platforms**. There is no per-platform patch set. So all three
platform builds must come from the *same fork commit*, or the platforms silently
diverge in behaviour.

---

## 3. The carry-set (canonical inventory)

Every one of these must survive every upgrade. Verified against
`rebuild-7778-procreq-plus-transparency` on 2026-09-09.

| # | Change | Layer | Files | Platform |
|---|---|---|---|---|
| 1 | `BeginWindowDrag` — native drag from an HTCLIENT region | B | `include/views/cef_window.h`, `libcef/browser/views/window_impl.{h,cc}` | All |
| 2 | Renderer-side transparency (Blink base-bg override) | B | `libcef/renderer/blink_glue.{h,cc}`, `libcef/renderer/browser_config.h`, `libcef/renderer/render_manager.cc`, `libcef/common/mojom/cef.mojom`, `libcef/browser/browser_platform_delegate.cc`, `libcef/browser/browser_info_manager.cc` | All (**macOS-critical**) |
| 3 | `rwhv_background_opaque_check` | A | `content/browser/renderer_host/render_widget_host_view_base.cc` | **macOS-critical**, no-op elsewhere |
| 4 | `views_caption_rightclick_passthrough` | A | `ui/views/widget/desktop_aura/window_event_filter_linux.cc` | **Linux only** |
| 5 | `agentmux_process_requirement` | A | `base/apple/mach_port_rendezvous_mac.cc` | **macOS 26 only** |

Notes that have already caused confusion:

- **#2 and #3 are one feature in two layers.** Transparency does not work with
  either half alone. #3 was introduced as `mac_rwhv_transparent_background.patch`
  in PR #4 and *replaced* by `rwhv_background_opaque_check.patch` in PR #5 — if
  you find the old name on a branch, that branch predates the codex-review fix.
- **#4 is Linux-only** despite the generic name. It patches
  `window_event_filter_linux.cc`.
- **#5 is not in the drag branch lineage at all**, which is exactly why the two
  branches had to be merged and why the gap opened.

---

## 4. Branch model — the rules

The root cause of §1.1 is branch reuse after merge. The repo-level rule for
this lives in [`CLAUDE.md`](../../CLAUDE.md) under Git Workflow:

> **Never reuse a branch after its PR is merged.**

It was added by the same PR as this document — it was *not* previously written
down anywhere in the repo, which is part of why §1.1 happened. It applies to
`agentmuxai/cef` as much as to this repo.

**R1 — One integration branch per Chromium milestone.** `fork/<milestone>`
(`fork/7778`, `fork/7977`). This is the *only* branch anyone builds or releases
from. It is upstream CEF's branch plus our carry-set, nothing else.

**R2 — Feature branches are single-use.** Once `agentmux/<ms>-<topic>` is merged
into `fork/<ms>`, it is dead. Follow-up work starts a new branch off
`fork/<ms>`, never off the merged branch. This alone would have prevented §1.1.

**R3 — Never build or release from a feature branch.** The July build used
`agentmux/7778-drag-rightclick-and-transparency` directly. It produced a correct
artifact *by luck* — that branch happened to be ahead. Building only from
`fork/<ms>` makes R1 self-enforcing: if the integration branch is incomplete,
you find out immediately rather than two months later.

**R4 — Merge the whole carry-set before cutting a release, not piecemeal.** The
152 work is split across `7977-drag-rightclick-and-transparency` and
`7977-process-requirement`. Merging one without the other reproduces §1.1
exactly. Merge both, then verify §5.

**R5 — `fork/<ms>` must never lose a carry-set item.** It is append-only with
respect to §3. If an upgrade legitimately drops one (upstream absorbed it), that
requires an explicit commit saying so, referencing the upstream change.

---

## 5. Verifying a branch is complete

Run this against any `fork/<ms>` before building or releasing from it. It is
cheap and it is the only thing that catches Layer B gaps.

```bash
# From the cef checkout. BR is the branch under test.
BR=fork/7778

# MANDATORY. These branches get force-updated mid-port; a stale ref produces a
# confident, wrong answer. See section 1.2 -- this exact step being skipped is
# what put a false claim in an earlier revision of this doc.
git fetch fork --prune

# Layer A -- patch files present AND registered in patch.cfg
for p in rwhv_background_opaque_check views_caption_rightclick_passthrough \
         agentmux_process_requirement; do
  git cat-file -e "${BR}:patch/patches/$p.patch" 2>/dev/null \
    && git show "${BR}:patch/patch.cfg" | grep -q "'name': '$p'" \
    && echo "OK   $p" || echo "MISS $p"
done

# Layer B -- probe identifiers, not filenames.
git show "${BR}:include/views/cef_window.h" | grep -q BeginWindowDrag \
  && echo "OK   BeginWindowDrag" || echo "MISS BeginWindowDrag"

# Transparency is a SEVEN-file cascade and must be probed as a unit. Checking
# only blink_glue + mojom passes on a partial port while browser-side
# propagation or render_manager.cc is still missing -- the gate would then
# approve the very regression it exists to catch.
probe() {  # probe <file> <identifier>
  git show "${BR}:$1" 2>/dev/null | grep -q "$2" \
    && echo "OK   $1" || echo "MISS $1"
}
probe libcef/renderer/blink_glue.h              SetBaseBackgroundColorOverrideTransparent
probe libcef/renderer/blink_glue.cc             SetBaseBackgroundColorOverrideTransparent
probe libcef/renderer/browser_config.h          background_transparent
probe libcef/renderer/render_manager.cc         background_transparent
probe libcef/common/mojom/cef.mojom             background_transparent
probe libcef/browser/browser_platform_delegate.cc  background_transparent
probe libcef/browser/browser_info_manager.cc    background_transparent
```

Expect **11 `OK`** against a complete `fork/<ms>`.

On a *feature* branch, `MISS agentmux_process_requirement` is normal rather than
a defect -- it lives on `agentmux/<ms>-process-requirement`. That is the R4
split, and it is why this block is meaningful only against the integration
branch.

Keep the `${BR}:` braces — this is not stylistic, and it is shell-dependent.
In **zsh** (the macOS default, so what these snippets usually get run in),
`"$BR:libcef/..."` applies `:l` as a history-style modifier and resolves
`fork/7778ibcef/...` — a false MISS on exactly the two probes that matter most.
**bash expands the braced and unbraced forms identically**, so this reproduces
for only some readers, which is worse than a consistent break.

The modifier letters that bite include `a e h l q r t u`. Verified in zsh with
`BR=fork/7778`:

```
$BR:libcef/x    -> fork/7778ibcef/x     ( :l  lowercase )
$BR:head/x      -> forkead/x            ( :h  dirname   )
$BR:tail/x      -> 7778ail/x            ( :t  basename  )
$BR:upper/x     -> FORK/7778pper/x      ( :u  uppercase )
$BR:include/x   -> fork/7778:include/x  ( :i  not a modifier - safe )
$BR:patch/x     -> fork/7778:patch/x    ( :p  not a modifier - safe )
```

`:include` and `:patch` being safe is why only the two `libcef` probes broke,
and why this survived a first reading.

**Probe identifiers, not filenames.** All of these files exist upstream. The 152
gap in §1.2 is invisible to a file-existence check and obvious to an identifier
check.

**Also confirm the integration branch is a superset of whatever last shipped:**

```bash
# Must print nothing. Anything printed is in the shipped artifact but not in
# the branch you are about to build — i.e. a regression you are about to ship.
git log --oneline "$LAST_SHIPPED_COMMIT" --not "fork/$MS"
```

That one command, run in September, would have caught §1.1 in a second.

---

## 6. Upgrade runbook (per Chromium milestone, all three platforms)

1. **Create `fork/<new-ms>`** from upstream CEF's branch for that milestone.
2. **Port the carry-set** in §3. One commit per item, message
   `agentmux: port <item> to <ms> (Chromium <N>)` — the 7977 branch already
   follows this convention and it makes §5 auditable.
   - Layer A: patches are context-sensitive and *will* need rebasing against new
     Chromium source. Re-generate rather than force-apply.
   - Layer B: usually clean cherry-picks; re-check API signatures that Chromium
     changed underneath.
3. **Merge every feature branch into `fork/<new-ms>`** (R4). Do not build from
   the feature branches.
4. **Run §5 against `fork/<new-ms>`.** All **11** probes must print `OK`.
5. **Build all three platforms from that one commit** — record the commit SHA.
6. **Verify the built artifacts** (§7), per platform.
7. **Cut three tags and update the pins together** (§8).

---

## 7. Verifying the built artifact, per platform

A branch being correct does not prove the build consumed it. Both failure modes
in §1 were about drift between intent and artifact, so check the binary.

### 7.1 `patcher.py` silent-failure (all platforms)

`patcher.py` takes **no arguments** in normal mode — it reads `patch/patch.cfg`
itself and accepts only `--patch-file` / `--patch-dir`. Passing `--root-dir`
makes it exit non-zero, and a wrapper script has swallowed exactly that as a
non-fatal `WARN` and continued. Fixed in the build docs by PR #3113; the trap is
worth restating because the failure applies **zero of 112 patches** and builds
anyway.

Two further traps, both hit in practice:

- `--patch-file` takes the patch **name**, not a path. It appends `.patch`
  itself, so `--patch-file patch/patches/foo.patch` looks for `foo.patch.patch`
  and fails.
- After running it, `git status --porcelain` in the Chromium tree must show
  **hundreds** of modified files (444 for 7778). A clean tree here means zero
  patches applied — that is the silent-failure signature, not success.

### 7.2 Symbol probes on the built binary

Our patch symbols are **local, not exported**. Use full `nm`. `nm -gU`
(external-only) will miss every one of them and report a false negative.

```bash
# macOS
BIN="Chromium Embedded Framework.framework/Versions/Current/Chromium Embedded Framework"
nm "$BIN" | grep BeginWindowDrag      # expect __ZN13CefWindowImpl15BeginWindowDragEv (type 't')

# Layer A landed at all? These come only from CEF's Chromium-side patches.
for s in IsUniqueForCEF IsAlloyContents HasExternalParent; do
  echo "$s: $(nm "$BIN" | grep -c "$s")"     # each must be > 0
done
```

`libcef.dll` (Windows) and `libcef.so` (Linux) take the same approach with
`dumpbin /symbols` and `nm -C` respectively. The `IsUniqueForCEF` /
`IsAlloyContents` / `HasExternalParent` triple is the useful generic probe: those
symbols exist **only** if CEF's Chromium-side patches were applied, so they
distinguish "patched build" from "pristine Chromium build" on any platform.

### 7.3 Platform-specific functional checks

| Platform | Check |
|---|---|
| **macOS** | Frameless window drags from an HTCLIENT region (#1); window background is actually transparent, not white (#2+#3); renderer does not crash-loop on macOS 26 (#5); H.264/HEVC playback works (codec flags) |
| **Linux** | Right-click in the caption area passes through (#4); drag (#1); H.264 playback (codec build) |
| **Windows** | Drag (#1); transparency (#2) |

---

## 8. Release pinning across three platforms

All three pins live in one place — `release.yml`'s `cef-runtime-pins` job — which
is correct and was a deliberate fix (PR #3086 and its follow-up). Keep it that
way; do not reintroduce per-job literals.

Current pins:

```
WIN_TAG="cef-windows-x86_64-148.0.7778.180"
LINUX_TAG="cef-linux-x86_64-148.0.7778.180-codecs"
MACOS_TAG="cef-macos-arm64-148.23.23-codecs"
```

**Known sharp edge: the three tags use two different version schemes.** Windows
and Linux carry the *Chromium* version (`148.0.7778.180`); macOS carries the
*CEF* version (`148.23.23`). The job cross-checks only the leading milestone
(`148`), which is the one component the schemes agree on. That check is real but
weak: **two tags can agree on `148` and still come from different fork commits
with different carry-sets.** That is exactly the divergence §1 is about, and
nothing currently catches it.

Practice until that is fixed:

- **P1 — Build all three platforms from one fork commit, and record that SHA** in
  the release PR body. It is the only thing that ties the three tags together.
- **P2 — Bump all three pins in the same PR.** A PR touching one pin should be
  treated as incomplete unless it says explicitly why the others stay.
- **P3 — Re-run §5 against the fork commit** when bumping pins, not just §7.

A worthwhile follow-up (not yet done): have the release job assert that all three
tags resolve to the same `agentmuxai/cef` commit, which would make P1–P3
mechanical instead of a convention.

---

## 9. Checklists

**Before merging a PR into `fork/<ms>`:**
- [ ] Branched off `fork/<ms>`, not off an already-merged feature branch (R2)
- [ ] `git log --oneline <last-shipped> --not fork/<ms>` prints nothing (§5)

**Before building a release framework:**
- [ ] Building from `fork/<ms>`, not a feature branch (R3)
- [ ] All feature branches for this milestone are merged (R4)
- [ ] All **11** §5 probes print `OK`
- [ ] `patcher.py` run with no args; Chromium tree shows hundreds of modified files (§7.1)

**Before updating the pins:**
- [ ] All three platforms built from one recorded fork commit (P1)
- [ ] §7.2 symbol probes pass on each artifact, using full `nm`
- [ ] §7.3 functional checks pass per platform
- [ ] All three pins bumped together (P2)
