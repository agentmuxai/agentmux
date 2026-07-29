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
- **Branch:** `agentmux/7778-drag-rightclick-and-transparency`
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
| Size (unstripped) | 547 MB |
| Version (`Info.plist` `CFBundleShortVersionString`) | 148.23.23.0 |
| `CEF_VERSION` (`cef_version.h`) | `148.23.23-rebuild-7778-codecs.3533+g6c570e2+chromium-148.0.7778.180` |
| Released tag | `cef-macos-arm64-148.23.23-codecs` (adds `proprietary_codecs` etc. — see `docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md`) |
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
# CEF version from Info.plist CFBundleShortVersionString (e.g. 148.23.23).
CEF_VERSION="148.23.23"
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

gh release create "cef-macos-arm64-${CEF_VERSION}${TAG_SUFFIX}" --repo agentmuxai/cef \
  --title "Patched CEF framework — macOS arm64 CEF ${CEF_VERSION}" \
  --notes "BeginWindowDrag + drag-rightclick + transparency. Branch: agentmux/7778-drag-rightclick-and-transparency. Unstripped (~547 MB); packager strips at bundle time." \
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
> 2026-07-28: every release created without an explicit `--target` picks up
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
