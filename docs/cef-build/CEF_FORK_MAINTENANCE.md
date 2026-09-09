# CEF Fork Maintenance — Branch Model and Upgrade Practice

**Status:** living
**Applies to:** `agentmuxai/cef` (our CEF fork), all three platforms
**Companion docs:** [build-patched-libcef.md](./build-patched-libcef.md) (macOS/Linux),
[build-patched-cef-windows.md](./build-patched-cef-windows.md),
[build-patched-framework-macos.md](./build-patched-framework-macos.md)

We maintain a fork of CEF carrying **four** AgentMux-specific changes — spanning
18 CEF source files and 3 Chromium-side patches (§3). Every Chromium
milestone upgrade has to carry that set forward, for Windows, Linux and macOS
together. This doc is the standing practice for doing that without losing pieces.

It exists because we lost pieces twice.

---

## 1. The two failures this prevents

Both are **silent**. Neither produced an error, a conflict, or a failed build.
That is the defining property of this class of bug: you find out when a user
reports a feature that used to work.

### 1.1 The forward-merge gap (2026-07-01 → 2026-09-09, found in production path)

`agentmux/7778-drag-rightclick-and-transparency` was merged into `agentmuxai/7778` via
PR #3 on Jun 24. Then PRs #4 and #5 added two **more** commits to that same
branch on Jul 1 — the renderer-side transparency work and the macOS
white-substitution fix. Nobody merged the branch forward a second time.

```
2720ba103  (Jun 24)  drag branch
    │
    ├──► merged into agentmuxai/7778 as 2d7833dec   (PR #3, Jun 24 18:57)
    │         │
    │         └──► 5387e4974  (PR #7, Sep 8)  ── agentmuxai/7778 tip
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
`agentmuxai/7778`. It only bit when someone went to `agentmuxai/7778` to pick up the
`process_requirement` P0 fix — the reasonable, obvious move. Every signal pointed
that way: `agentmuxai/7778` is the integration branch, it is named for the Chromium
version, and it carried the newest commit (Sep 8 vs Jul 1).

**Newer-looking is not the same as superset.** Building from `agentmuxai/7778` would
have fixed the renderer crash-loop and simultaneously regressed macOS window
transparency, with nothing anywhere reporting a problem.

### 1.2 Always re-fetch before you judge a branch

An earlier revision of this doc claimed the Chromium 152 port had dropped the
renderer-side transparency work. **That was wrong**, and the way it was wrong is
itself worth keeping.

The probe was real, but it ran against a stale local ref.
`agentmux/7977-drag-rightclick-and-transparency` had been **force-updated**
(`d6fe3d449` -> `55edc030a`). Two existing documents already said the port was
done -- `docs/reports/REPORT_CEF_UPGRADE_PHASE_A_RECON_2026_09_08.md` and
`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` -- and were not
consulted.

**Re-verified against the full 21-probe gate** (the first pass used only the
then-current 7 transparency probes, which could not have spoken for the 8
browser-side files §3 later added -- the same almost-complete-reads-as-complete
trap, one level up):

| Branch | Result |
|---|---|
| `agentmux/7977-drag-rightclick-and-transparency` | **20 OK, 1 MISS** |
| `agentmux/7977-process-requirement` | supplies the 1 missing patch |
| **`7977`** (the integration branch) | **0 OK, 21 MISS** |

So the 152 *port* is genuinely complete across the two feature branches --
including all 8 browser-side files. But **`7977` itself carries none of it**,
because neither branch has been merged. Per R3 that is the branch a release
would be built from, and per R4 both feature branches must land there first.
This is §1.1 exactly, caught before the fact instead of two months after.

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
- **When the gate changes, re-run every claim the old gate made.** The
  "complete" verdict above was first reached with 7 transparency probes and
  survived unchanged when §3 grew to 18 files -- so for a while it asserted
  something no probe had actually checked. A completeness claim is only ever as
  strong as the inventory current *at the moment it was made*, and expanding the
  inventory silently invalidates every earlier verdict.

The single `MISS` on the drag branch is the R4 case rather than a defect:
`agentmux_process_requirement` lives on `agentmux/7977-process-requirement`.
Both branches must be merged into `7977`. That split is precisely what produced
§1.1 on 7778, which is why R4 exists.

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
| **Detection** | Loud (`.rej` files, non-zero exit) — *unless* the invocation is wrong, see §7.1 | **Silent.** Nothing checks. |

Layer B is where both failures happened. There is no tool that notices a missing
libcef commit — the tree still compiles, because our additions are additive.

A corollary that matters for the upgrade runbook: `patch.cfg` is a **single file
shared by all three platforms**. There is no per-platform patch set. So all three
platform builds must come from the *same fork commit*, or the platforms silently
diverge in behaviour.

---

## 3. The carry-set (canonical inventory)

**Four features.** Two of them are Layer B only, one is Layer A only, and one
spans both — which is why a feature count and a file count are different
questions and both get stated here:

| Feature | Layer | Files |
|---|---|---|
| Drag (`BeginWindowDrag`) | B | 3 |
| Transparency | B + A | 15 + 1 patch |
| Right-click passthrough | A | 1 patch (Linux) |
| Process requirement | A | 1 patch (macOS 26) |

That totals **18 hand-written CEF source files** differing between upstream 7778
and the fork, plus **3 Chromium-side patches**. Measured, not estimated:

```bash
git diff --name-only <upstream-base> <branch> -- \
    'libcef/**' 'include/views/**' 'include/internal/**' \
  | grep -vE 'capi/|libcef_dll/'      # exclude generated wrappers
```

The 18 agrees with `docs/reports/REPORT_CEF_UPGRADE_PHASE_A_RECON_2026_09_08.md`
§5b, which reached it independently while scoping the 152 port.

> An earlier revision of this table listed only 10 of the 18, omitting the entire
> browser-side half of the transparency cascade. The §5 gate probed those 10 and
> reported a clean bill of health, so a branch missing eight fork-modified files
> would have passed. Caught by Codex review on PR #3121. The lesson is §2's: for
> Layer B nothing fails loudly, so an inventory that is *almost* complete reads
> exactly like one that is complete.

### Layer B — CEF source (18 files, carried by the branch itself)

| Feature | Files | Probe identifier |
|---|---|---|
| **Drag** (3) | `include/views/cef_window.h` | `BeginWindowDrag` |
| | `libcef/browser/views/window_impl.h` | `BeginWindowDrag` |
| | `libcef/browser/views/window_impl.cc` | `BeginWindowDrag` |
| **Transparency — renderer** (5) | `libcef/renderer/blink_glue.h` | `SetBaseBackgroundColorOverrideTransparent` |
| | `libcef/renderer/blink_glue.cc` | `SetBaseBackgroundColorOverrideTransparent` |
| | `libcef/renderer/browser_config.h` | `background_transparent` |
| | `libcef/renderer/render_manager.cc` | `background_transparent` |
| | `libcef/common/mojom/cef.mojom` | `background_transparent` |
| **Transparency — browser** (10) | `libcef/browser/browser_info_manager.cc` | `background_transparent` |
| | `libcef/browser/browser_platform_delegate.cc` | `background_transparent` |
| | `libcef/browser/browser_platform_delegate_create.cc` | `is_views_hosted` |
| | `libcef/browser/browser_host_base.cc` | `IsWindowless() \|\| is_views_hosted()` |
| | `libcef/browser/context.cc` | `is_transparent` |
| | `libcef/browser/context.h` | `transparent_state` |
| | `libcef/browser/views/browser_view_impl.cc` | `ApplyToCurrentRWHView` |
| | `libcef/browser/views/browser_view_impl.h` | `LayerTreeHost` |
| | `libcef/browser/views/window_view.cc` | `CalculateRenderPasses` |
| | `include/internal/cef_types.h` | `or a frameless window` |

Every probe identifier above is **absent upstream and present in the fork** —
asserted for all 18, so each one actually discriminates rather than matching
code that was already there.

### Layer A — Chromium-side patches (3)

| Patch | Patches | Platform |
|---|---|---|
| `rwhv_background_opaque_check` | `content/browser/renderer_host/render_widget_host_view_base.cc` | **macOS-critical**, no-op elsewhere |
| `views_caption_rightclick_passthrough` | `ui/views/widget/desktop_aura/window_event_filter_linux.cc` | **Linux only** |
| `agentmux_process_requirement` | `base/apple/mach_port_rendezvous_mac.cc` | **macOS 26 only** |

Notes that have already caused confusion:

- **Transparency is one feature spanning both layers and 16 files** (15 Layer B +
  `rwhv_background_opaque_check`). It works with none of the halves alone.
  `SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md` §3 says these commits "should
  be ported as a unit"; treat any partial port as broken, not partially working.
- The Layer A patch was introduced as `mac_rwhv_transparent_background.patch` in
  PR #4 and *replaced* by `rwhv_background_opaque_check.patch` in PR #5. Finding
  the old name on a branch means it predates the codex-review fix.
- **`views_caption_rightclick_passthrough` is Linux-only** despite the generic
  name — it patches `window_event_filter_linux.cc`.
- **`agentmux_process_requirement` is not in the drag branch lineage at all**,
  which is why the two branches had to be merged and how §1.1's gap opened.

---

## 4. Branch model — the rules

> **Notation.** The integration branch is named for the milestone alone --
> `7778`, `7977` (`refs/heads/7778` on `agentmuxai/cef`). Where this doc writes
> `agentmuxai/7778` that is `<remote>/<branch>`, not a branch called
> `agentmuxai/7778`. An earlier revision wrote `fork/7778`, using one checkout's
> local remote name; a reviewer reasonably read that as the branch name and
> concluded the checkout commands were wrong. If the notation can mislead a
> careful reader it can mislead an operator, so it is spelled out here.


The root cause of §1.1 is branch reuse after merge. The repo-level rule for
this lives in [`CLAUDE.md`](../../CLAUDE.md) under Git Workflow:

> **Never reuse a branch after its PR is merged.**

It was added by the same PR as this document — it was *not* previously written
down anywhere in the repo, which is part of why §1.1 happened. It applies to
`agentmuxai/cef` as much as to this repo.

**R1 — One integration branch per Chromium milestone**, named for the milestone
alone: `7778`, `7977` (written `agentmuxai/7778` when referring to the
remote-tracking ref). This is the *only* branch anyone builds or releases
from. It is upstream CEF's branch plus our carry-set, nothing else.

**R2 — Feature branches are single-use.** Once `agentmux/<ms>-<topic>` is merged
into `agentmuxai/<ms>`, it is dead. Follow-up work starts a new branch off
`agentmuxai/<ms>`, never off the merged branch. This alone would have prevented §1.1.

**R3 — Never build or release from a feature branch.** The July build used
`agentmux/7778-drag-rightclick-and-transparency` directly. It produced a correct
artifact *by luck* — that branch happened to be ahead. Building only from
`agentmuxai/<ms>` makes R1 self-enforcing: if the integration branch is incomplete,
you find out immediately rather than two months later.

**R4 — Merge the whole carry-set before cutting a release, not piecemeal.** The
152 work is split across `7977-drag-rightclick-and-transparency` and
`7977-process-requirement`. Merging one without the other reproduces §1.1
exactly. Merge both, then verify §5.

**R5 — `agentmuxai/<ms>` must never lose a carry-set item.** It is append-only with
respect to §3. If an upgrade legitimately drops one (upstream absorbed it), that
requires an explicit commit saying so, referencing the upstream change.

---

## 5. Verifying a branch is complete

Run this against any `agentmuxai/<ms>` before building or releasing from it. It is
cheap and it is the only thing that catches Layer B gaps.

```bash
# Paste into a shell, then run:
#   cef_verify                      # integration branch 7778 on remote 'agentmuxai'
#   cef_verify 7977 fork            # milestone 7977, remote named 'fork'
#   cef_verify a1b2c3d4...          # an exact SHA -- see section 8, P3
#
# A function, not a bare script: a top-level `exit 1` would close the shell of
# anyone who pasted this, which is a poor reward for following the doc.
cef_verify() {
  local REF="${1:-7778}"
  local REMOTE="${2:-${REMOTE:-agentmuxai}}"
  local BR

  if [[ "$REF" =~ ^[0-9]+$ ]]; then
    # A bare milestone number means the integration branch on REMOTE.
    # REMOTE is whatever YOU named the agentmuxai/cef remote. The companion
    # runbooks add it as 'agentmuxai'; some checkouts call it 'fork'. Hard-coding
    # it either errors out or silently resolves a same-named ref from an
    # unrelated remote -- the class of failure this gate exists to catch.
    git remote get-url "$REMOTE" >/dev/null 2>&1 || {
      echo "No remote '$REMOTE'. Pass it: cef_verify $REF <remote>  (git remote -v)" >&2
      return 1
    }
    # MANDATORY. These branches get force-updated mid-port; a stale ref gives a
    # confident, wrong answer. Section 1.2 -- skipping this put a false claim in
    # an earlier revision of this very doc.
    git fetch "$REMOTE" --prune || return 1
    BR="$REMOTE/$REF"
  else
    # Anything else is used verbatim: a tag, a full SHA, a local branch. This is
    # what section 8's P3 needs -- verifying a RELEASE ARTIFACT means checking
    # the commit it was built from, not whatever the branch has become since. A
    # fix landed after the build would otherwise make the gate pass for an
    # artifact that does not contain it.
    BR="$REF"
  fi

  git rev-parse --verify -q "${BR}^{commit}" >/dev/null || {
    echo "Cannot resolve '$BR'. The integration branch is named for the milestone alone." >&2
    return 1
  }

  local ok=0 miss=0
  _probe() {  # _probe <path> <identifier-absent-upstream>
    if git show "${BR}:$1" 2>/dev/null | grep -qF "$2"; then
      ok=$((ok+1));   printf 'OK   %s\n' "$1"
    else
      miss=$((miss+1)); printf 'MISS %s\n' "$1"
    fi
  }

  # ---- Layer A: patch file present AND registered in patch.cfg ----
  local name
  for name in rwhv_background_opaque_check views_caption_rightclick_passthrough \
              agentmux_process_requirement; do
    if git cat-file -e "${BR}:patch/patches/${name}.patch" 2>/dev/null \
       && git show "${BR}:patch/patch.cfg" | grep -q "'name': '${name}'"; then
      ok=$((ok+1));   printf 'OK   patch/%s\n' "$name"
    else
      miss=$((miss+1)); printf 'MISS patch/%s\n' "$name"
    fi
  done

  # ---- Layer B: all 18 fork-modified CEF sources (section 3) ----
  # Identifiers, not filenames: every one of these files exists upstream, so a
  # file-existence check proves nothing. Each identifier is asserted absent
  # upstream and present in the fork, so it actually discriminates.
  _probe include/views/cef_window.h                        BeginWindowDrag
  _probe libcef/browser/views/window_impl.h                BeginWindowDrag
  _probe libcef/browser/views/window_impl.cc               BeginWindowDrag
  _probe libcef/renderer/blink_glue.h                      SetBaseBackgroundColorOverrideTransparent
  _probe libcef/renderer/blink_glue.cc                     SetBaseBackgroundColorOverrideTransparent
  _probe libcef/renderer/browser_config.h                  background_transparent
  _probe libcef/renderer/render_manager.cc                 background_transparent
  _probe libcef/common/mojom/cef.mojom                     background_transparent
  _probe libcef/browser/browser_info_manager.cc            background_transparent
  _probe libcef/browser/browser_platform_delegate.cc       background_transparent
  _probe libcef/browser/browser_platform_delegate_create.cc is_views_hosted
  _probe libcef/browser/browser_host_base.cc               'IsWindowless() || is_views_hosted()'
  _probe libcef/browser/context.cc                         is_transparent
  _probe libcef/browser/context.h                          transparent_state
  _probe libcef/browser/views/browser_view_impl.cc         ApplyToCurrentRWHView
  _probe libcef/browser/views/browser_view_impl.h          LayerTreeHost
  _probe libcef/browser/views/window_view.cc               CalculateRenderPasses
  _probe include/internal/cef_types.h                      'or a frameless window'

  unset -f _probe
  echo "--- $BR: $ok OK, $miss MISS (expect 21 OK, 0 MISS) ---"
  [ "$miss" -eq 0 ]
}
```

Expect **21 `OK`** against a complete `agentmuxai/<ms>`.

On a *feature* branch, `MISS agentmux_process_requirement` is normal rather than
a defect -- it lives on `agentmux/<ms>-process-requirement`. That is the R4
split, and it is why this block is meaningful only against the integration
branch.

Keep the `${BR}:` braces — this is not stylistic, and it is shell-dependent.
In **zsh** (the macOS default, so what these snippets usually get run in),
`"$BR:libcef/..."` applies `:l` as a history-style modifier and resolves
`agentmuxai/7778ibcef/...` — a false MISS, silently.

**The gate above avoids this structurally**, and that is deliberate: `_probe`
takes the path as an *argument*, so the expansion is `${BR}:$1`, and zsh applies
modifiers only to *literal* text after the colon. Verified:

```
${BR}:$f                                  -> agentmuxai/7778:libcef/renderer/blink_glue.h
$BR:$f                                    -> agentmuxai/7778:libcef/renderer/blink_glue.h   (also fine)
$BR:libcef/renderer/blink_glue.h          -> agentmuxai/7778ibcef/renderer/blink_glue.h     (broken)
```

So the hazard only bites if you **inline a path literally**. Do not — and if you
do anyway, 16 of the 21 probes break: every `libcef/`-prefixed one. The
remaining 5 are safe purely by first letter (`include/` -> `:i`, `patch/` ->
`:p` are not modifiers), so a silent *partial* pass is the default outcome
rather than an obvious total failure.

**bash expands the braced and unbraced forms identically**, so this reproduces
for only some readers, which is worse than a consistent break.

The modifier letters that bite include `a e h l q r t u`. Real output, produced
by running this with `BR=agentmuxai/7778` rather than hand-written:

```
$BR:libcef/x    -> agentmuxai/7778ibcef/x     ( :l  lowercase )
$BR:head/x      -> agentmuxaiead/x            ( :h  dirname   )
$BR:tail/x      -> 7778ail/x                  ( :t  basename  )
$BR:upper/x     -> AGENTMUXAI/7778pper/x      ( :u  uppercase )
$BR:include/x   -> agentmuxai/7778:include/x  ( :i  not a modifier - safe )
$BR:patch/x     -> agentmuxai/7778:patch/x    ( :p  not a modifier - safe )
```

The safe/unsafe split falls on the first letter of the path, not on anything
meaningful — which is why this survived a first reading. At the time the bug
was found the block probed only two `libcef/` paths; expanding the cascade to
seven widened the blast radius without changing the cause.

**Probe identifiers, not filenames.** All of these files exist upstream. The 152
gap in §1.2 is invisible to a file-existence check and obvious to an identifier
check.

**Also confirm the integration branch is a superset of whatever last shipped:**

```bash
# Must print nothing. Anything printed is in the shipped artifact but not in
# the branch you are about to build — i.e. a regression you are about to ship.
git log --oneline "$LAST_SHIPPED_COMMIT" --not "$REMOTE/$MS"
```

That one command, run in September, would have caught §1.1 in a second.

---

## 6. Upgrade runbook (per Chromium milestone, all three platforms)

1. **Create the integration branch `<new-ms>`** on `agentmuxai/cef`, from upstream
   CEF's branch for that milestone.
2. **Port the carry-set** in §3. One commit per item, message
   `agentmux: port <item> to <ms> (Chromium <N>)` — the 7977 branch already
   follows this convention and it makes §5 auditable.
   - Layer A: patches are context-sensitive and *will* need rebasing against new
     Chromium source. Re-generate rather than force-apply.
   - Layer B: usually clean cherry-picks; re-check API signatures that Chromium
     changed underneath.
3. **Merge every feature branch into `agentmuxai/<new-ms>`** (R4). Do not build from
   the feature branches.
4. **Run §5 against `agentmuxai/<new-ms>`.** All **21** probes must print `OK`.
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

### 7.2b Patches that add no symbol — differential compile

Symbol probes only work for patches that introduce a symbol. **Two of the three
Layer A patches do not**: `agentmux_process_requirement` rewrites the body of an
existing function, and `rwhv_background_opaque_check` changes one call inside
one. `nm` cannot see either, so §7.2 alone cannot tell you whether they made it
into the artifact.

Compile the file twice — from the working tree, and from pristine upstream — and
compare both against the object that was actually linked:

```bash
cd out/Release_GN_arm64
SRC=base/apple/mach_port_rendezvous_mac.cc
OBJ=obj/base/base/mach_port_rendezvous_mac.o

git -C ../.. show HEAD:$SRC > /tmp/unpatched.cc          # pristine upstream
CMD=$(ninja -C . -t commands "$OBJ" | tail -1)           # the real compile line

eval "${CMD//$OBJ//tmp/patched.o}"                       # from the working tree
eval "$(echo "$CMD" | sed "s#$OBJ#/tmp/unpatched.o#; s#\.\./\.\./$SRC#/tmp/unpatched.cc#")"

cmp -s /tmp/unpatched.o "$OBJ" && echo 'FAIL: shipped object is UNPATCHED'
cmp -s /tmp/patched.o   "$OBJ" && echo 'OK: shipped object matches patched source'
```

Both comparisons matter. `shipped == patched` alone is weak if the patch happens
to compile to the same bytes as upstream; `shipped != unpatched` is what proves
the patch actually changed the output. Confirmed for both patches on the
2026-09-09 macOS build.

Do not try to read this off a disassembly. Attempting exactly that on
`GetPeerValidationPolicy` produced a confident *wrong* answer — the patched
function had been inlined and the symbol at that address was a neighbouring one,
so the control flow looked unpatched. The byte comparison is unambiguous where
eyeballing assembly is not.

---

### 7.3 Platform-specific functional checks

| Platform | Check |
|---|---|
| **macOS** | **Drag**: frameless window drags from an HTCLIENT region. **Transparency**: window background is actually transparent, not white — needs *both* layers, see §3. **Process requirement**: renderer does not crash-loop on macOS 26. Plus H.264/HEVC playback (codec flags). |
| **Linux** | **Right-click passthrough**: right-click in the caption area passes through. **Drag**. Plus H.264 playback (codec build). |
| **Windows** | **Drag**. **Transparency**. |

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

**Before merging a PR into `agentmuxai/<ms>`:**
- [ ] Branched off `agentmuxai/<ms>`, not off an already-merged feature branch (R2)
- [ ] `git log --oneline <last-shipped> --not agentmuxai/<ms>` prints nothing (§5)

**Before building a release framework:**
- [ ] Building from `agentmuxai/<ms>`, not a feature branch (R3)
- [ ] All feature branches for this milestone are merged (R4)
- [ ] All **21** §5 probes print `OK`
- [ ] `patcher.py` run with no args; Chromium tree shows hundreds of modified files (§7.1)

**Before updating the pins:**
- [ ] All three platforms built from one recorded fork commit (P1)
- [ ] §7.2 symbol probes pass on each artifact, using full `nm`
- [ ] §7.2b differential compile passes for the two patches `nm` cannot see
      (`agentmux_process_requirement`, `rwhv_background_opaque_check`)
- [ ] §7.3 functional checks pass per platform
- [ ] All three pins bumped together (P2)
