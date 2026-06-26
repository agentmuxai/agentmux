# SPEC: agentmux-builder — macOS + Linux CI Release Workflows

**Date:** 2026-06-24
**Status:** Draft
**Repo:** agentmuxai/agentmux-builder (private)
**Tracks:** issue #1718 (nightly cross-platform artifacts Phase B)

---

## 1. Problem

`agentmux-builder` currently only has `build-windows.yml`. macOS DMGs and Linux
AppImages are still cut locally and uploaded by hand. The blockers described in
`ci-nightly-artifacts.yml` are:

- **macOS**: "`package:macos` hard-requires a Developer ID certificate — needs
  secrets/keychain plumbing before it can run in CI."
- **Linux**: "`libcef.so` BeginWindowDrag patch gate blocks packaging unless
  `AGENTMUX_SKIP_CEF_PATCH_CHECK=1`; the upstream cef-dll-sys cache lacks the
  patch."

This spec resolves both blockers and adds two new workflows to `agentmux-builder`.

---

## 2. macOS Workflow (`build-macos.yml`)

### 2.1 Trigger

```yaml
on:
  workflow_dispatch:
    inputs:
      ref:
        description: "agentmux ref to build (tag / branch / SHA)"
        required: true
        default: main
      release-tag:
        description: "GitHub release tag to upload the DMG to (blank = skip)"
        required: false
        default: ""
  repository_dispatch:
    types: [build-macos]
```

### 2.2 Runner

`macos-latest` (GitHub-hosted, Apple Silicon arm64). GitHub's hosted macOS
runners ship Xcode, `codesign`, `xcrun notarytool`, `hdiutil`, and `sips` —
all required by `scripts/package-macos.sh`.

### 2.3 Steps

```
1. Checkout agentmuxai/agentmux at <ref> (AGENTMUX_CHECKOUT_TOKEN)
2. Install Rust (stable) + dtolnay/rust-toolchain
3. Install Node 22 + npm ci (A5AF_PACKAGES_TOKEN for @a5af packages)
4. Import Developer ID certificate into a temporary keychain
5. Store notarytool credentials in the temporary keychain
6. task package:macos -- "$OUTDIR"
7. gh release upload <release-tag> <DMG> (AGENTMUX_RELEASE_TOKEN)
8. Cleanup: delete temp keychain
```

### 2.4 Certificate Import (Step 4)

The standard CI pattern for Apple codesigning — creates a short-lived keychain
that is torn down after the job:

```bash
KEYCHAIN=build-$(uuidgen).keychain
KEYCHAIN_PASS=$(uuidgen)

security create-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN"
security default-keychain -s "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN"
security set-keychain-settings -t 3600 -u "$KEYCHAIN"

echo "$APPLE_CERTIFICATE" | base64 --decode > cert.p12
security import cert.p12 \
    -k "$KEYCHAIN" \
    -P "$APPLE_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security

# Allow codesign to access the key without a UI prompt
security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s -k "$KEYCHAIN_PASS" "$KEYCHAIN"

rm -f cert.p12
```

`package-macos.sh` calls `security find-identity -v -p codesigning` to auto-
detect the Developer ID identity, so no explicit cert name needs to be threaded
through once it's in the keychain.

### 2.5 Notarytool Credential Store (Step 5)

```bash
xcrun notarytool store-credentials "notarytool" \
  --apple-id  "$APPLE_ID" \
  --password  "$APPLE_PASSWORD" \
  --team-id   "$APPLE_TEAM_ID" \
  --keychain "$KEYCHAIN"
```

`package-macos.sh` defaults to `NOTARY_PROFILE=notarytool` and passes
`--keychain-profile notarytool` to `xcrun notarytool submit`. No script changes
required.

### 2.6 Build Command

```bash
OUTDIR="$GITHUB_WORKSPACE/artifacts"
mkdir -p "$OUTDIR"
task package:macos -- "$OUTDIR"
```

`task package:macos` runs `build:host`, `build:backend`, `build:frontend`,
`bundle`, then `scripts/package-macos.sh`. Output:
`AgentMux_{VERSION}_arm64.dmg`.

### 2.7 Upload

```bash
DMG=$(ls "$OUTDIR"/AgentMux_*_arm64.dmg | head -1)
gh release upload "$RELEASE_TAG" "$DMG" \
  --repo agentmuxai/agentmux --clobber
```

Only runs when `release-tag` input is non-empty.

### 2.8 Secrets Used

| Secret | Value | Already in agentmux-builder |
|--------|-------|------------------------------|
| `AGENTMUX_CHECKOUT_TOKEN` | PAT (repo) to clone agentmuxai/agentmux | ✅ |
| `AGENTMUX_RELEASE_TOKEN` | PAT (repo) to upload release assets | ✅ |
| `A5AF_PACKAGES_TOKEN` | npm GitHub Packages token | ✅ |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID .p12 | ✅ |
| `APPLE_CERTIFICATE_PASSWORD` | .p12 export password | ✅ |
| `APPLE_SIGNING_IDENTITY` | Full cert name (optional — auto-detected) | ✅ |
| `APPLE_ID` | Apple ID email for notarytool | ✅ |
| `APPLE_PASSWORD` | App-specific password | ✅ |
| `APPLE_TEAM_ID` | 10-char Apple Team ID | ✅ |

All secrets are already present in `agentmux-builder` per the README. No new
secrets need to be created.

---

## 3. Linux Workflow (`build-linux.yml`)

### 3.1 The Patched libcef.so Problem

The `build-appimage-linux.sh` script requires a **patched** `libcef.so` built
from the `agentmuxai/cef` fork (branch `agentmux/7778-drag-rightclick-and-
transparency`) that adds `CefWindow::BeginWindowDrag()`. The upstream prebuilt
libcef.so bundled by `cef-dll-sys` lacks this patch; bundling it ships with
broken left-click window drag on Wayland.

Building libcef.so from source in CI is not feasible (~3-6 hours wall-clock on
a 32-core box, 99 GB disk, Chrome toolchain). The solution: **store a pre-built
patched `libcef.so` as a release asset in `agentmuxai/cef`** and download it in
CI via `AGENTMUX_CEF_RUNTIME_DIR`.

### 3.2 Patched libcef.so Artifact

**Location:** `agentmuxai/cef` — GitHub release tagged
`cef-linux-x86_64-<CEF_VERSION>` (e.g. `cef-linux-x86_64-148.2.7`).

**Artifact name:** `cef-linux-x86_64-<CEF_VERSION>.tar.gz`

**Contents:**
```
libcef.so           ← the unstripped patched build (~613 MB; packager strips it)
icudtl.dat
snapshot_blob.bin
v8_context_snapshot.bin
chrome_100_percent.pak
chrome_200_percent.pak
resources.pak
headless_command_resources.pak
libEGL.so
libGLESv2.so
libvk_swiftshader.so
libvulkan.so.1
vk_swiftshader_icd.json
chrome-sandbox
chrome_crashpad_handler
locales/
```

This archive is produced once per CEF version bump (not per AgentMux release) by
whoever builds the patched CEF locally, and uploaded with:

```bash
gh release create "cef-linux-x86_64-148.2.7" \
  --repo agentmuxai/cef \
  --title "Patched libcef.so — Linux x86_64 CEF 148.2.7" \
  cef-linux-x86_64-148.2.7.tar.gz
```

A new repo secret `CEF_RUNTIME_TOKEN` (PAT with `read:packages` or `contents:read`
on `agentmuxai/cef`) is added to `agentmux-builder` to authenticate the download.

### 3.3 Trigger

```yaml
on:
  workflow_dispatch:
    inputs:
      ref:
        description: "agentmux ref to build (tag / branch / SHA)"
        required: true
        default: main
      release-tag:
        description: "GitHub release tag to upload the AppImage to (blank = skip)"
        required: false
        default: ""
      cef-runtime-tag:
        description: "agentmuxai/cef release tag for the patched libcef.so"
        required: false
        default: ""   # blank = auto-detect from latest release
  repository_dispatch:
    types: [build-linux]
    # client_payload: { ref, release_tag, cef_runtime_tag }
```

### 3.4 Runner

`ubuntu-22.04` (GitHub-hosted, x86_64). Matches the libcef.so build target and
the AppImage runtime baseline.

### 3.5 Steps

```
1.  Checkout agentmuxai/agentmux at <ref> (AGENTMUX_CHECKOUT_TOKEN)
2.  Install Rust (stable) + dtolnay/rust-toolchain
3.  Install Node 22 + npm ci (A5AF_PACKAGES_TOKEN)
4.  Install system deps (ninja-build cmake libwayland-dev libxkbcommon-dev libgtk-3-dev)
5.  Download + extract patched libcef.so from agentmuxai/cef release
6.  Set AGENTMUX_CEF_RUNTIME_DIR to the extracted directory
7.  Download appimagetool to ~/.local/bin/appimagetool
8.  task package:linux -- "$OUTDIR"
9.  gh release upload <release-tag> <AppImage> (AGENTMUX_RELEASE_TOKEN)
```

### 3.6 libcef.so Download (Step 5)

```bash
# Resolve tag (explicit input or latest release in agentmuxai/cef)
CEF_TAG="${CEF_RUNTIME_TAG:-}"
if [ -z "$CEF_TAG" ]; then
  CEF_TAG=$(gh release list --repo agentmuxai/cef --limit 1 \
              --json tagName --jq '.[0].tagName')
fi

mkdir -p "$HOME/cef-runtime"
gh release download "$CEF_TAG" \
  --repo agentmuxai/cef \
  --pattern "*.tar.gz" \
  --dir "$HOME/cef-runtime" \
  --clobber

tar -xzf "$HOME/cef-runtime"/*.tar.gz -C "$HOME/cef-runtime"
# Remove the archive; keep the extracted tree
rm "$HOME/cef-runtime"/*.tar.gz

echo "AGENTMUX_CEF_RUNTIME_DIR=$HOME/cef-runtime" >> "$GITHUB_ENV"
```

With `AGENTMUX_CEF_RUNTIME_DIR` set, `scripts/resolve-cef-runtime.sh` returns
this path as its first candidate (explicit override) and skips the cargo cache
fallback. The unstripped libcef.so passes `verify-cef-patch.sh`; the packager
then strips it inside the AppDir.

### 3.7 appimagetool Download (Step 7)

```bash
mkdir -p "$HOME/.local/bin"
curl -L -o "$HOME/.local/bin/appimagetool" \
  "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
chmod +x "$HOME/.local/bin/appimagetool"
# AppImages need FUSE; on GitHub runners use --appimage-extract-and-run instead
export APPIMAGETOOL_OPTS="--appimage-extract-and-run"
# Re-export appimagetool itself via extract-and-run
APPIMAGETOOL="$HOME/.local/bin/appimagetool --appimage-extract-and-run"
```

GitHub-hosted Ubuntu runners do not have FUSE. `appimagetool` supports
`--appimage-extract-and-run` to run without mounting.

### 3.8 Build Command

```bash
OUTDIR="$GITHUB_WORKSPACE/artifacts"
mkdir -p "$OUTDIR"
APPIMAGETOOL="$HOME/.local/bin/appimagetool --appimage-extract-and-run" \
  task package:linux -- "$OUTDIR"
```

Output: `AgentMux_{VERSION}_amd64.AppImage`.

### 3.9 Upload

```bash
APPIMAGE=$(ls "$OUTDIR"/AgentMux_*_amd64.AppImage | head -1)
gh release upload "$RELEASE_TAG" "$APPIMAGE" \
  --repo agentmuxai/agentmux --clobber
```

Only runs when `release-tag` input is non-empty.

### 3.10 Secrets Used

| Secret | Value | Already in agentmux-builder |
|--------|-------|------------------------------|
| `AGENTMUX_CHECKOUT_TOKEN` | PAT to clone agentmuxai/agentmux | ✅ |
| `AGENTMUX_RELEASE_TOKEN` | PAT to upload release assets | ✅ |
| `A5AF_PACKAGES_TOKEN` | npm GitHub Packages token | ✅ |
| `CEF_RUNTIME_TOKEN` | PAT (`contents:read`) for agentmuxai/cef | ❌ **new** |

One new secret: `CEF_RUNTIME_TOKEN` — a fine-grained PAT scoped to
`agentmuxai/cef` with `contents:read` permission so the workflow can download
release assets from the private CEF repo. Add it to `agentmux-builder` repo
settings → Secrets.

---

## 4. Release Flow (All Three Platforms)

### 4.1 Pre-release checklist

1. `chore: release vX.Y.Z` PR merged → tag `vX.Y.Z` pushed to `agentmuxai/agentmux`
2. GitHub Release created for `vX.Y.Z` (draft or pre-release; Windows build
   attaches the signed installer)
3. Patched `libcef.so` for the current CEF version is available as a release in
   `agentmuxai/cef` (only changes when CEF version bumps)

### 4.2 Triggering the three builds

Trigger all three from `agentmuxai/agentmux-builder` → Actions tab, or via
`repository_dispatch` from a release automation script:

```bash
VERSION="v0.49.1"

# Windows (build-windows.yml)
gh workflow run build-windows.yml \
  --repo agentmuxai/agentmux-builder \
  -f ref="$VERSION" -f release-tag="$VERSION"

# macOS (build-macos.yml)
gh workflow run build-macos.yml \
  --repo agentmuxai/agentmux-builder \
  -f ref="$VERSION" -f release-tag="$VERSION"

# Linux (build-linux.yml)
gh workflow run build-linux.yml \
  --repo agentmuxai/agentmux-builder \
  -f ref="$VERSION" -f release-tag="$VERSION"
```

All three upload directly to the `agentmuxai/agentmux` release for `$VERSION`
using `AGENTMUX_RELEASE_TOKEN`.

### 4.3 Expected release assets after all three complete

| File | Platform | Size (approx) |
|------|----------|---------------|
| `agentmux-X.Y.Z-x64-portable.zip` | Windows | ~169 MB |
| `AgentMux-X.Y.Z-x64-setup.exe` | Windows | ~130 MB |
| `AgentMux_X.Y.Z_arm64.dmg` | macOS (Apple Silicon) | ~140 MB |
| `AgentMux_X.Y.Z_amd64.AppImage` | Linux x86_64 | ~230 MB |

---

## 5. Not in Scope

- **Intel Mac (x86_64)**: all builds target Apple Silicon (arm64). `cef-dll-sys`
  resolves the aarch64 framework; x86_64 support would need a separate CEF build.
- **Linux arm64**: `build-appimage-linux.sh` targets x86_64; arm64 AppImage deferred.
- **Windows code signing** (SignPath): deferred per `docs/windows-code-signing.md`.
- **macOS App Store**: direct distribution only; notarized Developer ID path.
- **Auto-trigger on tag push**: the workflows are `workflow_dispatch` + 
  `repository_dispatch` only. A tag-push trigger (listen for `vX.Y.Z` tags on 
  `agentmux`) would require cross-repo webhook wiring — left for a future
  `release-orchestrator.yml`.
- **Nightly macOS/Linux artifacts** (non-release builds): the nightly 
  `ci-nightly-artifacts.yml` in the main repo continues to produce Windows only.
  Extending it to macOS/Linux would require cert secrets on the public repo's
  runner — feasible but out of scope for this spec (the secrets live in the
  private `agentmux-builder` precisely to avoid that).
