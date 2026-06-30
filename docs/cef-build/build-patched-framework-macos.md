# Building the Patched CEF Framework for AgentMux (macOS arm64)

**Audience:** AgentMux maintainers building a macOS release that needs native
window drag / floating-pane edge-resize.
**Time:** First build ~3-6 hours wall-clock (CPU-bound chromium compile).
**Disk:** ~99 GB chromium working tree + build output.
**Output:** A `Chromium Embedded Framework.framework` (~545 MB unstripped) with the
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

The framework currently referenced by the release pipeline (verified 2026-06-29):

| Property | Value |
|----------|-------|
| Path | `~/cef-build/chromium/chromium/src/out/Release_GN_arm64/Chromium Embedded Framework.framework` |
| Arch | Mach-O 64-bit arm64 |
| Size (unstripped) | 545 MB |
| Version (`Info.plist` `CFBundleShortVersionString`) | 148.0.9 |
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
third_party/ninja/ninja -j 12 -l 16 -C out/Release_GN_arm64 cef
```

Output: `out/Release_GN_arm64/Chromium Embedded Framework.framework` (~545 MB
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
# CEF version from Info.plist CFBundleShortVersionString (e.g. 148.0.9).
CEF_VERSION="148.0.9"

bash /path/to/agentmux/scripts/verify-cef-framework-darwin.sh "$CEF_OUT"   # must exit 0

cd "$CEF_OUT"
# tar preserves the Versions/Current symlink chain on macOS by default. Verify after
# upload that the extracted tree still has the
# `Chromium Embedded Framework -> Versions/Current/Chromium Embedded Framework`
# symlink; if BSD tar mangles it, use `ditto -c -k --keepParent` (zip) instead and
# adjust the CI extract step to `ditto -x -k`.
tar -czf "cef-macos-arm64-${CEF_VERSION}.tar.gz" "Chromium Embedded Framework.framework"

gh release create "cef-macos-arm64-${CEF_VERSION}" --repo agentmuxai/cef \
  --title "Patched CEF framework — macOS arm64 CEF ${CEF_VERSION}" \
  --notes "BeginWindowDrag + drag-rightclick + transparency. Branch: agentmux/7778-drag-rightclick-and-transparency. Unstripped (~545 MB); packager strips at bundle time." \
  "cef-macos-arm64-${CEF_VERSION}.tar.gz"
```

**Naming convention** (parallels Linux `cef-linux-x86_64-<ver>`):
- Tag: `cef-macos-arm64-<CEF_VERSION>`
- Asset: `cef-macos-arm64-<CEF_VERSION>.tar.gz`

`build-macos.yml` auto-detects the latest `cef-macos-arm64-*` release (or takes an
explicit `cef-runtime-tag` input), downloads + caches it, sets
`AGENTMUX_CEF_RUNTIME_DIR_DARWIN`, and the package gate verifies the patch.

---

## Version skew with Linux (follow-up)

The Linux release is at CEF `148.0.20`; this macOS framework is `148.0.9`. Functional
parity for `BeginWindowDrag` is fine, but matching versions is cleaner for support.
**Tracked follow-up:** rebuild macOS arm64 at `148.0.20` (and with
`is_official_build=true` for the size win) and re-cut the release so both platforms
converge.
