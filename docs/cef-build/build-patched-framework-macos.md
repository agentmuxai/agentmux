# Building the Patched CEF Framework for AgentMux (macOS arm64)

**Audience:** AgentMux maintainers building a macOS release that needs native
window drag / floating-pane edge-resize.
**Time:** First build ~3-6 hours wall-clock (CPU-bound chromium compile).
**Disk:** ~99 GB chromium working tree + build output.
**Output:** A `Chromium Embedded Framework.framework` (~547 MB unstripped) with the
AgentMux `BeginWindowDrag` patch that upstream CEF / its prebuilt binary
distribution lack.

This is the macOS counterpart to `build-patched-libcef.md` (Linux). Read that doc
for the shared chromium/CEF/depot_tools mechanics; this doc covers only the
macOS-specific deltas.

---

## Why this exists

The upstream `cef-dll-sys` framework lacks `CefWindow::BeginWindowDrag()`. Without
it, macOS native window drag no-ops — which is exactly why the floating-pane header
drag and edge-resize fall back to JS-polled `get/set_window_position` /
`set_window_rect` workarounds (`floating-pane-workspace.tsx`,
`jsDrivenDrag = isMacOS() || isLinux()`). Shipping the patched framework is the
prerequisite for moving macOS onto the native drag path.

The patches live in the same fork/branch as Linux:
- **Repo:** https://github.com/agentmuxai/cef
- **Branch:** `7778` — the **integration** branch for the milestone. Never build
  from a feature branch such as `agentmux/7778-drag-rightclick-and-transparency`:
  it may look newer and still be missing part of the carry-set, which is exactly
  how the 2026-07 transparency gap happened. Verify with §5 of
  [CEF_FORK_MAINTENANCE.md](./CEF_FORK_MAINTENANCE.md) (expect **21 `OK`**)
  before building; if it fails, fix the integration branch rather than building
  around it.
- **Base:** Chromium 148 (CEF branch 7778)
- **Rust binding:** `AgentU-asaf/cef-rs@agentmux/148-begin-window-drag` (pinned in
  `Cargo.toml` `[patch]`; the binding's `_cef_window_t` carries `begin_window_drag`
  for both linux and macos arch dirs).

---

## What the verified artifact looks like

The framework currently referenced by the release pipeline (verified
2026-07-28 — this table had drifted out of date even before today's codec
work; the actual latest published tag by 2026-07-02 was already `148.23.21`,
not the `148.0.9` this table previously claimed):

| Property | Value |
|----------|-------|
| Path | `~/cef-build/chromium/chromium/src/out/Release_GN_arm64/Chromium Embedded Framework.framework` |
| Arch | Mach-O 64-bit arm64 |
| Size (unstripped) | 428 MB binary; 181 MB as the released `.tar.gz` |
| Version (`Info.plist` `CFBundleShortVersionString`) | 148.23.25.0 |
| Built from | `agentmuxai/cef` `1bee8b7da` (ancestor of `7778`) — the release's `--target`, so the tag records the build commit |
| Released tag | `cef-macos-arm64-148.23.25-codecs` (2026-09-09; adds the macOS 26 renderer fix `agentmux_process_requirement` and the renderer-side transparency work over `148.23.23-codecs`) |
| Patch symbol | `__ZN13CefWindowImpl15BeginWindowDragEv` (local symbol, `nm` type `t`) |

> ⚠️ The patch symbol is **local**, not exported. Verify with full `nm` —
> `nm -gU` (external-only) will MISS it. `scripts/verify-cef-framework-darwin.sh`
> handles this; don't "optimize" it to `nm -gU`.

---

## Prerequisites (macOS deltas)

- macOS arm64 host (Apple Silicon) with Xcode + command-line tools.
- ≥ 32 GB RAM, ≥ 120 GB free disk.
- `depot_tools` on PATH; the chromium hooks pull the macOS toolchain automatically.

The depot_tools / automate-git / fork-checkout / patcher steps are **identical to
Linux** — follow `build-patched-libcef.md` §1–§3, with `--branch=7778`.

---

## Configure the build (macOS args)

The canonical GN args are version-controlled at **`scripts/cef-build/args-darwin.gn`**
— the configuration that produced the verified arm64 framework. Key deltas from the
Linux `args.gn`:

- `target_cpu="arm64"`
- `is_official_build=false` (the captured build; see the size follow-up note in the
  args file — flipping this to `true` + `use_thin_lto=true` should roughly halve the
  binary, tracked as a follow-up)
- `symbol_level=1` (stripped by `package-macos.sh` at bundle time)
- no Linux-only knobs (`use_sysroot`, `use_qt*`)

```bash
cd ~/cef-build/chromium/chromium/src
# Regenerate the gitignored C-API wrappers FIRST (same gotcha as Linux)
( cd cef && python3 tools/translator.py --root-dir . )
cp /path/to/agentmux/scripts/cef-build/args-darwin.gn out/Release_GN_arm64/args.gn
./buildtools/mac/gn gen out/Release_GN_arm64
```

---

## Build

```bash
cd ~/cef-build/chromium/chromium/src
# macOS doesn't need the systemd-run cgroup isolation Linux uses; ninja directly.
# Build the framework target (NOT the phony `cef` meta-target, which won't relink
# after a source-only change).
third_party/ninja/ninja -j 12 -l 16 -C out/Release_GN_arm64 cef_framework
```

Output: `out/Release_GN_arm64/Chromium Embedded Framework.framework` (~547 MB
unstripped). Do **not** strip it here — `package-macos.sh` strips at bundle time, and
the patch-verify gate keys on the local symbol that `strip -S -x` removes.

---

## Verify the patch

```bash
bash /path/to/agentmux/scripts/verify-cef-framework-darwin.sh \
  ~/cef-build/chromium/chromium/src/out/Release_GN_arm64
# exit 0 = patched · exit 1 = unpatched upstream · exit 2 = stripped/unverifiable
```

- `task bundle:darwin` runs this **advisorily** (a warning — so `task dev` still
  works on the upstream cef-dll-sys fallback).
- `scripts/package-macos.sh` runs it as a **hard release gate** before signing
  (override with `AGENTMUX_SKIP_CEF_PATCH_CHECK=1` for a deliberate upstream-CEF
  package). The gate runs on `dist/Frameworks/` — unstripped at that point; the
  `strip -S -x` happens later inside the assembled `.app`.

---

## Using the built framework in AgentMux

### Option A: Default location
If you built at `~/cef-build/darwin/<arch>/` (the standard layout
`resolve-cef-runtime-darwin.sh` looks for at tier 2), `task bundle:darwin` picks it
up automatically. Copy/symlink the framework there:
```bash
mkdir -p ~/cef-build/darwin/aarch64
ditto ~/cef-build/chromium/chromium/src/out/Release_GN_arm64/"Chromium Embedded Framework.framework" \
      ~/cef-build/darwin/aarch64/"Chromium Embedded Framework.framework"
```

### Option B: Explicit override
```bash
export AGENTMUX_CEF_RUNTIME_DIR_DARWIN=~/cef-build/chromium/chromium/src/out/Release_GN_arm64
task bundle:darwin
```
`resolve-cef-runtime-darwin.sh` treats this as a hard requirement (tier 1) — a typo
fails fast rather than silently falling through to the unpatched cargo cache.

---

## Package + upload as a GitHub release (for CI)

Local builds resolve the framework from `~/cef-build/...` directly, so this step is
**only for CI** — which has no build tree and pulls the patched framework from a
release in `agentmuxai/cef` (consumed by `build-macos.yml`).

**Do NOT strip first.** `package-macos.sh` strips at bundle time, and the verify gate
keys on the local symbol that `strip` removes — upload the **unstripped** framework.

```bash
CEF_OUT=~/cef-build/chromium/chromium/src/out/Release_GN_arm64
# CEF version from Info.plist CFBundleShortVersionString. READ IT FROM THE
# BUILD -- do not copy this example, it is a placeholder that has gone stale
# before. `/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" \
#   "$CEF_OUT/Chromium Embedded Framework.framework/Versions/A/Resources/Info.plist"`
CEF_VERSION="148.23.25"
# Append a suffix (e.g. -codecs) whenever the build adds a distinguishing
# feature over the last release at the same numeric CEF_VERSION — see
# docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md and the
# "auto-detection is unreliable" warning below for why this matters more than
# it looks like it should.
TAG_SUFFIX="-codecs"

bash /path/to/agentmux/scripts/verify-cef-framework-darwin.sh "$CEF_OUT"   # must exit 0

cd "$CEF_OUT"
# tar preserves the Versions/Current symlink chain on macOS by default. Verify after
# upload that the extracted tree still has the
# `Chromium Embedded Framework -> Versions/Current/Chromium Embedded Framework`
# symlink; if BSD tar mangles it, use `ditto -c -k --keepParent` (zip) instead and
# adjust the CI extract step to `ditto -x -k`.
tar -czf "cef-macos-arm64-${CEF_VERSION}${TAG_SUFFIX}.tar.gz" "Chromium Embedded Framework.framework"

# Record the exact fork commit this artifact came from -- the only thing tying
# the three platforms' tags together. See CEF_FORK_MAINTENANCE.md section 8 (P1).
#
# Locate the fork clone rather than assuming a path. The mirrored copy under
# chromium/src/cef is created with `rsync --exclude=.git`, so it has no .git of
# its own -- and `git -C` on such a directory does not fail, it silently WALKS UP
# and answers from the enclosing Chromium checkout, returning CHROMIUM's HEAD. A
# SHA that does not exist in agentmuxai/cef would end up in --target and in the
# notes, with no error. Layouts also differ: some trees clone the fork directly
# at chromium/src/cef, others mirror into it from an outer clone.
#
# An explicit CEF_CLONE is a HARD requirement: validated, then used or fatal.
# Falling through to a standard path when the override is wrong would record a
# DIFFERENT checkout's HEAD -- a valid-looking but unrelated fork SHA -- and
# look entirely successful.
if [ -n "${CEF_CLONE:-}" ]; then
  [ "$(git -C "$CEF_CLONE" rev-parse --show-toplevel 2>/dev/null)" = "$(cd "$CEF_CLONE" 2>/dev/null && pwd -P)" ] \
    && git -C "$CEF_CLONE" remote -v 2>/dev/null | grep -q 'agentmuxai/cef' \
    || { echo "FATAL: CEF_CLONE=$CEF_CLONE is not an agentmuxai/cef git root." >&2; CEF_CLONE=; }
else
  # No override: probe. First candidate that is BOTH its own git root AND has
  # the agentmuxai/cef remote. A .git-less mirror is not a git root, so this
  # rejects the case where `git -C` would walk up into the Chromium checkout.
  for _c in "$HOME/cef-build/chromium/chromium/src/cef" \
            "$HOME/cef-build/chromium/cef" \
            "$HOME/cef-build/chromium_git/cef"; do
    [ "$(git -C "$_c" rev-parse --show-toplevel 2>/dev/null)" = "$(cd "$_c" 2>/dev/null && pwd -P)" ] || continue
    git -C "$_c" remote -v 2>/dev/null | grep -q 'agentmuxai/cef' || continue
    CEF_CLONE="$_c"; break
  done
fi
echo "fork clone: ${CEF_CLONE:-<none>}"

# FULL sha: `gh release create --target` takes a branch or a full commit SHA.
#
# The :? on CEF_CLONE is load-bearing, not decoration. `git -C "" rev-parse HEAD`
# does NOT fail -- git treats an empty -C as "unchanged directory", exits 0, and
# answers for the CURRENT directory, which by this point is inside the Chromium
# checkout. An unmatched probe would then yield Chromium's HEAD: a real-looking
# SHA, so a downstream emptiness check never fires. Failing at expansion time,
# before git runs at all, is what removes that path.
CEF_FORK_SHA=$(git -C "${CEF_CLONE:?FATAL: no agentmuxai/cef clone found (a .git-less mirror does not count); set CEF_CLONE}" rev-parse HEAD)
: "${CEF_FORK_SHA:?refusing to publish without a recorded fork commit}"

gh release create "cef-macos-arm64-${CEF_VERSION}${TAG_SUFFIX}" --repo agentmuxai/cef \
  --target "${CEF_FORK_SHA}" \
  --title "Patched CEF framework — macOS arm64 CEF ${CEF_VERSION}" \
  --notes "BeginWindowDrag + drag-rightclick + transparency. Built from agentmuxai/cef ${CEF_FORK_SHA:0:12} (branch 7778). Unstripped (~547 MB); packager strips at bundle time." \
  "cef-macos-arm64-${CEF_VERSION}${TAG_SUFFIX}.tar.gz"
```

**Naming convention** (parallels Linux `cef-linux-x86_64-<ver>`):
- Tag: `cef-macos-arm64-<CEF_VERSION>[-suffix]`
- Asset: `cef-macos-arm64-<CEF_VERSION>[-suffix].tar.gz`

`build-macos.yml` / `ci-nightly-artifacts.yml` auto-detect a `cef-macos-arm64-*`
release via `gh release list --json tagName --jq '[.[] | select(startswith(...))][0]'`
(or take an explicit `cef-runtime-tag` input), download + cache it, set
`AGENTMUX_CEF_RUNTIME_DIR_DARWIN`, and the package gate verifies the patch.

> ⚠️ **"Latest" here is not what you'd assume.** `gh release list`'s default
> order is *not* reliably publish-time-descending on this fork — confirmed
> 2026-07-28. **The release recipe above now passes `--target "${CEF_FORK_SHA}"`,
> which addresses this at the source**; the verification below stays as a
> belt-and-braces check, and remains necessary for the tags cut before that
> change. Historically: every release created without an explicit `--target` picks up
> `agentmuxai/cef`'s frozen default-branch HEAD commit date as its `created_at`
> (not the actual `gh release create` call time), and `gh release list`
> appears to sort by `created_at`. Net effect: a brand-new release can sort
> **behind** an older one whose tag happened to target a newer commit, and
> `[0]` after the `select()` picks the wrong (stale) tag. This actually
> happened when cutting `cef-macos-arm64-148.23.23-codecs` — it initially
> lost to the older `cef-macos-arm64-148.23.21` in this exact query. **Always
> verify after cutting a release:**
> ```bash
> gh release list --repo agentmuxai/cef --limit 30 --json tagName \
>   --jq '[.[].tagName | select(startswith("cef-macos-arm64-"))][0]'
> ```
> If it doesn't print the tag you just created, CI will silently keep shipping
> the old framework. The reliable fix is deleting/retiring the superseding
> older tag(s) so there's no ambiguity for `[0]` to get wrong — not something
> a version bump or suffix alone fixes. See
> `docs/specs/STATUS_CEF_PROPRIETARY_CODECS_MACOS_2026_07_27.md` for the
> full incident and what was actually done about it for this release.

---

## Version skew with Linux (follow-up)

As of 2026-07-28 macOS is at CEF `148.23.23` (this doc's "verified artifact"
table above) — check the current Linux release
(`docs/cef-build/build-patched-libcef.md` or `gh release list --repo
agentmuxai/cef`) before assuming either platform's exact version, since both
move independently and this note will drift out of date the next time either
rebuilds. **Tracked follow-up:** align both platforms on the same CEF version
(and build macOS with `is_official_build=true` for the size win — see the
size-reduction follow-up note in `scripts/cef-build/args-darwin.gn`) and
re-cut the release so both platforms converge.
