# SPEC: Patched macOS CEF Framework — Release Pipeline + CI Wiring

**Date:** 2026-06-29
**Status:** Plan (awaiting review — no code written yet)
**Repos:** `agentmuxai/agentmux`, `agentmuxai/cef`
**Tracks:** macOS parity with the Linux patched-libcef pipeline
**Related:** `SPEC_BUILDER_MACOS_LINUX_CI_2026_06_24.md`,
`SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28.md`,
`patched-libcef-bundling-2026-05-08.md`, `build-patched-libcef.md`

---

## 1. Problem

**Linux** ships a *patched* CEF that adds `CefWindow::BeginWindowDrag()`
(+ drag-rightclick + transparency). **macOS does not** — the macOS DMG silently
ships the **upstream** CEF framework that lacks `BeginWindowDrag`. This is the
root cause of the floating-pane drag/resize fragility on macOS: with no native
`BeginWindowDrag`, the floater header drag and edge-resize are driven by
JS-polled `get/set_window_position` / `set_window_rect` workarounds
(`floating-pane-workspace.tsx`), which are racier than the native path Windows
and (patched) Linux use.

### 1.1 How Linux closes the gap (reference: `build-linux.yml`)

1. Downloads a pre-built patched `libcef.so` from an `agentmuxai/cef` **release**
   (`cef-linux-x86_64-<ver>.tar.gz`).
2. Sets `AGENTMUX_CEF_RUNTIME_DIR` → `scripts/resolve-cef-runtime.sh` injects it
   into `task bundle:linux`.
3. `scripts/verify-cef-patch.sh` gates the build so an unpatched `.so` can't ship.

### 1.2 What macOS does today (the gap)

- `build-macos.yml` runs `task package:macos` with **no CEF override**.
- `task bundle:darwin` → `scripts/resolve-cef-runtime-darwin.sh` falls through its
  three-tier resolution to the **`cef-dll-sys` cargo cache** (tier 3) = upstream
  framework, **no `BeginWindowDrag`**.
- There is **no macOS patch-verification gate**, so the unpatched framework ships
  silently.

### 1.3 What already exists (so this is wiring, not new architecture)

- `resolve-cef-runtime-darwin.sh` **already honors `AGENTMUX_CEF_RUNTIME_DIR_DARWIN`**
  (tier 1, hard-fail on a set-but-invalid path). The injection hook exists; it is
  simply never fed in CI.
- `task bundle:darwin` already `ditto`s the framework (preserving the
  `Versions/Current` symlink chain) and copies the GL dylibs next to the host exe.
- `package-macos.sh` already assumes "the patched CEF 148 framework" (helper-app
  spawning, mach-port peer disable).
- `cef-dll-sys` (fork `AgentU-asaf/cef-rs`, pinned in `Cargo.toml`) provides the
  Rust *bindings* exposing the `BeginWindowDrag` slot — but the **native framework
  binary** behind it is still upstream. The patched framework is the missing half.

---

## 2. The artifact already exists locally

A patched arm64 framework has already been built on this dev machine:

```
~/cef-build/chromium/chromium/src/out/Release_GN_arm64/Chromium Embedded Framework.framework
```

Verified 2026-06-29:

| Property | Value |
|----------|-------|
| Arch | Mach-O 64-bit arm64 |
| Size (unstripped) | **545 MB** |
| Version (`Info.plist` `CFBundleShortVersionString`) | **148.0.9.0** |
| Patch symbol | `__ZN13CefWindowImpl15BeginWindowDragEv` present |
| Symbol binding | **local** (`nm` type `t`, lowercase) — NOT exported |

> ⚠️ **Version skew:** the existing **Linux** release is CEF `148.0.20`; this
> macOS framework is `148.0.9`. Decide whether to (a) cut the macOS release at
> `148.0.9` as-is, or (b) rebuild macOS at `148.0.20` to match Linux. Functional
> parity is fine at 148.0.9 for `BeginWindowDrag`; matching versions is cleaner
> for support. **Recommendation:** ship `148.0.9` now to unblock, track a
> `148.0.20` rebuild as a follow-up so both platforms converge.

> ⚠️ **Verification must run UNSTRIPPED.** Because `BeginWindowDrag` is a *local*
> symbol, `nm -gU` (external-only) will not find it — only a full `nm` over an
> unstripped binary will. The release asset must therefore be uploaded
> **unstripped** (matching the Linux "packager strips it at bundle time"
> convention), and the verify gate must run **before** `package-macos.sh`'s strip
> step.

---

## 3. Work items

Four pieces. Pieces 1–2 + 4 are writable now; Piece 3 documents the (already-done)
build and the release-cut recipe.

### Piece 1 — Cut the `agentmuxai/cef` macOS release

The release asset, produced once per CEF version bump (not per AgentMux release):

```bash
cd ~/cef-build/chromium/chromium/src/out/Release_GN_arm64
# Tar the framework UNSTRIPPED, preserving the Versions/Current symlink chain.
# Use ditto (or gnutar with --no-mac-metadata off) so the symlinks survive.
tar -czf /tmp/cef-macos-arm64-148.0.9.tar.gz "Chromium Embedded Framework.framework"

gh release create cef-macos-arm64-148.0.9 \
  --repo agentmuxai/cef \
  --title "Patched CEF framework — macOS arm64 CEF 148.0.9" \
  --notes "BeginWindowDrag + drag-rightclick + transparency. Built from agentmux/7778-drag-rightclick-and-transparency. Unstripped (~545 MB); packager strips at bundle time." \
  /tmp/cef-macos-arm64-148.0.9.tar.gz
```

> Tarball preservation: the framework contains a `Versions/Current` symlink chain
> the CEF loader follows. `tar -czf` on macOS preserves symlinks by default; verify
> after upload that the extracted tree still has `Chromium Embedded Framework ->
> Versions/Current/Chromium Embedded Framework`. If BSD tar mangles it, fall back
> to `ditto -c -k --keepParent` (zip) and adjust the CI extract step to `ditto -x -k`.

**Naming convention** (parallels Linux `cef-linux-x86_64-<ver>`):
- Tag: `cef-macos-arm64-<CEF_VERSION>` → `cef-macos-arm64-148.0.9`
- Asset: `cef-macos-arm64-<CEF_VERSION>.tar.gz`

### Piece 2 — `build-macos.yml` CI wiring

Mirror the Linux CEF block. Insert **after** the npm-auth step, **before**
`task package:macos`.

**New workflow inputs** (parallel to Linux):
```yaml
workflow_dispatch.inputs.cef-runtime-tag:
  description: "agentmuxai/cef release tag for patched framework (blank = latest macOS)"
  required: false
  default: ""
repository_dispatch.client_payload.cef_runtime_tag   # threaded in the Resolve-inputs step
```

**New steps:**
1. **Resolve cef tag** (`id: cef`): if `cef_tag` input is blank, auto-detect —
   but **filter by the macOS prefix**, since `agentmuxai/cef` now holds both
   platforms:
   ```bash
   TAG=$(gh release list --repo agentmuxai/cef --limit 30 \
           --json tagName --jq '[.[].tagName | select(startswith("cef-macos-arm64-"))][0]')
   ```
   > Also fix `build-linux.yml`'s resolver the same way — its bare `--limit 1`
   > would now grab whichever platform released most recently. (Tracked as a
   > one-line follow-up in that file.)
2. **Cache** (`actions/cache@v4`, key `cef-runtime-darwin-<tag>`, path
   `$CEF_RUNTIME_DIR_DARWIN`).
3. **Download + extract** (only on cache miss), auth via `CEF_RUNTIME_TOKEN`:
   ```bash
   mkdir -p /tmp/cef-x "$CEF_RUNTIME_DIR_DARWIN"
   gh release download "${{ steps.cef.outputs.tag }}" \
     --repo agentmuxai/cef --pattern "*.tar.gz" --dir /tmp/cef-x --clobber
   tar -xzf /tmp/cef-x/*.tar.gz -C /tmp/cef-x
   FW=$(find /tmp/cef-x -name "Chromium Embedded Framework.framework" -maxdepth 3 | head -1)
   [ -z "$FW" ] && { echo "framework not found in tarball"; exit 1; }
   ditto "$FW" "$CEF_RUNTIME_DIR_DARWIN/Chromium Embedded Framework.framework"
   ```
4. **Export the override:**
   ```bash
   echo "AGENTMUX_CEF_RUNTIME_DIR_DARWIN=$CEF_RUNTIME_DIR_DARWIN" >> "$GITHUB_ENV"
   ```
   With this set, `resolve-cef-runtime-darwin.sh` returns it as tier 1 and never
   touches the upstream cargo cache.

**Job-level env:** `CEF_RUNTIME_DIR_DARWIN: ${{ github.workspace }}/cef-runtime-darwin`

**New secret:** `CEF_RUNTIME_TOKEN` (fine-grained PAT, `contents:read` on
`agentmuxai/cef`). Per `SPEC_BUILDER_MACOS_LINUX_CI` this already exists in the
builder repo for Linux; it must be present wherever `build-macos.yml` runs.

> **OPEN QUESTION — workflow home.** `build-macos.yml` currently lives in
> **this repo** (`.github/workflows/build-macos.yml`), but
> `SPEC_BUILDER_MACOS_LINUX_CI` describes the macOS/Linux release workflows as
> living in **`agentmuxai/agentmux-builder`**. These appear to have diverged.
> Confirm the canonical home before landing — the diff + the `CEF_RUNTIME_TOKEN`
> secret must go there. The diff below is written against this repo's copy.

### Piece 3 — `docs/cef-build/build-patched-framework-macos.md`

New doc paralleling `build-patched-libcef.md` (which is Linux-only). Captures the
already-done build so it's reproducible at the next CEF bump:

- **Source:** branch `agentmux/7778-drag-rightclick-and-transparency` in
  `agentmuxai/cef`.
- **GN args:** the macOS variant of `docs/cef-build/args.gn`
  (`target_os="mac"`, `target_cpu="arm64"`, `is_official_build=true`,
  `symbol_level=1`, `is_component_build=false`, …). Capture the actual
  `args.gn` used for the `Release_GN_arm64` build on this machine.
- **Output:** `out/Release_GN_arm64/Chromium Embedded Framework.framework`
  (545 MB unstripped).
- **Release-cut recipe:** the Piece 1 `tar` + `gh release create` block.
- **Note:** upload **unstripped** — both the verify gate (Piece 4) and the
  package strip depend on symbols being present at download time.

### Piece 4 — macOS patch-verify gate

New `scripts/verify-cef-framework-darwin.sh`, analogue of `verify-cef-patch.sh`:

- **Target:** the Mach-O dylib `"<framework>/Chromium Embedded Framework"`
  (resolve the `Versions/Current` symlink).
- **Detection:** `nm "<binary>" 2>/dev/null | grep -q 'BeginWindowDrag'`.
  Use **full `nm`** (whole symbol table) — NOT `nm -gU` — because the patch symbol
  is a *local* (`t`) symbol, not exported.
- **Exit codes** mirror Linux:
  - `0` — patched (`BeginWindowDrag` found)
  - `1` — UNPATCHED: symbol table present but no `BeginWindowDrag` (the real alarm)
  - `2` — cannot verify: stripped binary (no symbol table), missing file, or no
    `nm` available
- **Wire into `task bundle:darwin`** immediately after `resolve-cef-runtime-darwin.sh`
  yields `CEF_DIR`, **before** the `ditto` — and before `package-macos.sh`'s strip.
  Gate behind `AGENTMUX_SKIP_CEF_PATCH_CHECK` (same escape hatch as Linux) so a
  local dev intentionally on upstream CEF isn't blocked. In CI the gate is on by
  default → an unpatched framework hard-fails the build instead of shipping.

> A stripped framework returns exit 2 ("cannot verify"). Decide CI policy: treat
> exit 2 as failure in CI (we control the asset and require it unstripped) but as
> a soft warning locally (a dev may legitimately point at a stripped framework).

---

## 4. Why the bundle path already works (no `bundle:darwin` logic change)

`task bundle:darwin` resolves `CEF_DIR` via `resolve-cef-runtime-darwin.sh`,
checks `"$CEF_DIR/Chromium Embedded Framework.framework"` exists, then `ditto`s it
into `dist/Frameworks/` and copies `Libraries/*.dylib` next to the host exe. Once
`AGENTMUX_CEF_RUNTIME_DIR_DARWIN` points at the downloaded framework, this path is
unchanged except for the inserted verify call. No restructuring required.

---

## 5. Validation plan

1. **Local dry-run:** `AGENTMUX_CEF_RUNTIME_DIR_DARWIN=~/cef-build/chromium/chromium/src/out/Release_GN_arm64 task package:macos -- /tmp/out`
   → confirm the DMG's framework carries `BeginWindowDrag` (`nm` on the bundled
   binary pre-strip) and that the verify gate passes.
2. **Native drag smoke test:** with the patched framework bundled, confirm the
   floating-pane header drag + edge-resize can move to the native
   `BeginWindowDrag` path (separate follow-up: the frontend currently hard-codes
   `jsDrivenDrag = isMacOS() || isLinux()`; flipping macOS off is a *future* change
   gated on this framework shipping — out of scope here, but this release is its
   prerequisite).
3. **CI:** trigger `build-macos.yml` with `cef-runtime-tag=cef-macos-arm64-148.0.9`,
   confirm download+cache+verify+package all green and the DMG uploads.

---

## 6. Sequencing

1. **Piece 1** — cut the `cef-macos-arm64-148.0.9` release (unblocks everything;
   the artifact is already built locally).
2. **Piece 4** — verify script + `bundle:darwin` wiring (testable locally against
   the local framework immediately).
3. **Piece 2** — `build-macos.yml` wiring (testable only once Piece 1's release
   exists + `CEF_RUNTIME_TOKEN` is present in the workflow's repo).
4. **Piece 3** — build doc (parallel; documents Piece 1).
5. Update `SPEC_BUILDER_MACOS_LINUX_CI_2026_06_24.md` §2/§4.1/§5 to fold in the
   macOS patched-framework path (its §5 currently defers from-source macOS CEF —
   revise to "consume pre-built patched framework, like Linux").

---

## 7. Open questions (need answers before landing)

1. **Workflow home** — is `build-macos.yml` canonical in this repo or in
   `agentmux-builder`? (Determines where the diff + `CEF_RUNTIME_TOKEN` go.)
2. **CEF version** — ship macOS at `148.0.9` now, or rebuild to match Linux's
   `148.0.20`? (Recommendation: ship 148.0.9, track 148.0.20 convergence.)
3. **Stripped-framework CI policy** — verify-gate exit 2 = hard fail in CI? (Yes,
   recommended, since we require the unstripped asset.)
4. **Out of scope here, confirm tracked separately:** flipping
   `jsDrivenDrag` off for macOS once the patched framework ships in releases — the
   actual payoff of this work.

---

## 8. Not in scope

- Building the framework from source (already done; documented in Piece 3 for the
  next bump).
- Intel macOS (x86_64) — arm64 only, matching the rest of the macOS pipeline.
- Changing the frontend drag path (`jsDrivenDrag`) — a dependent follow-up.
- Windows — **correction (2026-07-26): this was stale/incorrect.** Windows
  does not ship a patched CEF; it uses the plain stock `cef-dll-sys`
  binary with zero override capability anywhere in the build/CI pipeline
  (confirmed at every layer — `Taskfile.yml`'s Windows tasks, every CI
  workflow's Windows job, and a full-repo grep for
  `AGENTMUX_CEF_RUNTIME_DIR`). Windows was excluded from *this* spec
  because it never needed the `BeginWindowDrag` patch specifically
  (native drag already works there via Win32) — not because it already
  has an equivalent patched build. See
  `docs/specs/SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md` for
  the first Windows custom-CEF build work, driven by a different need
  (proprietary codec support).
