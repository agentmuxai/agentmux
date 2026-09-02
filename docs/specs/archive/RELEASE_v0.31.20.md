# Release Spec: AgentMux v0.31.20

**Date:** 2026-03-03
**Version:** 0.31.20
**Goal:** Produce Windows, macOS, and Linux installers + portables from the fresh `agentmuxai/agentmux` repo.

---

## Problem

The existing CI workflow (`.github/workflows/tauri-build.yml`) is stale:
- References Go 1.23 and builds Go binaries (`go build ./cmd/server/main-server.go`)
- The backend was rewritten to 100% Rust months ago; Go code no longer exists
- The `create-release` job only triggers on tag pushes, but the workflow is `workflow_dispatch` only
- Binary copy step uses Go naming conventions, not Rust target triples

The workflow must be rewritten for the Rust backend before triggering a release.

---

## Release Artifacts

| Platform | Arch | Installer | Portable |
|----------|------|-----------|----------|
| Windows | x64 | NSIS `.exe` setup | `.zip` bundle |
| macOS | ARM64 (Apple Silicon) | `.dmg` | `.app` in `.tar.gz` |
| macOS | x64 (Intel) | `.dmg` | `.app` in `.tar.gz` |
| Linux | x64 | `.deb` | `.AppImage` |

Each platform produces two sidecar binaries:
- `agentmuxsrv-rs-{TARGET_TRIPLE}[.exe]` (backend server)
- `wsh-{TARGET_TRIPLE}[.exe]` (shell integration)

---

## Implementation Plan

### Step 1: Fix CI Workflow

Rewrite `.github/workflows/tauri-build.yml`:

**Remove:**
- `actions/setup-go@v5` step (Go is no longer used)
- "Build Go backend binaries" step (entire step)

**Replace with Rust backend build step:**
```yaml
- name: Build Rust backend binaries
  shell: bash
  run: |
    VERSION=$(node -p "require('./package.json').version")

    # Build agentmuxsrv-rs
    cargo build --release -p agentmuxsrv-rs --target ${{ matrix.target }}

    # Build wsh-rs
    cargo build --release -p wsh-rs --target ${{ matrix.target }}

    # Copy to dist/bin with Tauri sidecar naming
    mkdir -p src-tauri/binaries
    if [ "${{ matrix.platform }}" = "windows" ]; then
      cp target/${{ matrix.target }}/release/agentmuxsrv-rs.exe \
         src-tauri/binaries/agentmuxsrv-rs-${{ matrix.target }}.exe
      cp target/${{ matrix.target }}/release/wsh.exe \
         src-tauri/binaries/wsh-${{ matrix.target }}.exe
    else
      cp target/${{ matrix.target }}/release/agentmuxsrv-rs \
         src-tauri/binaries/agentmuxsrv-rs-${{ matrix.target }}
      cp target/${{ matrix.target }}/release/wsh \
         src-tauri/binaries/wsh-${{ matrix.target }}
    fi

    # Also copy versioned wsh for resource bundling
    mkdir -p src-tauri/binaries/bin
    if [ "${{ matrix.platform }}" = "windows" ]; then
      cp target/${{ matrix.target }}/release/wsh.exe \
         src-tauri/binaries/bin/wsh-${VERSION}-${{ matrix.platform }}.${{ matrix.arch }}.exe
    else
      cp target/${{ matrix.target }}/release/wsh \
         src-tauri/binaries/bin/wsh-${VERSION}-${{ matrix.platform }}.${{ matrix.arch }}
    fi

    ls -lh src-tauri/binaries/
    ls -lh src-tauri/binaries/bin/
```

**Remove** the separate "Copy sidecar binaries to Tauri" step (now integrated above).

**Update the Tauri build step:**
```yaml
- name: Build Tauri application
  uses: tauri-apps/tauri-action@v0
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
    TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
  with:
    tauriScript: npx tauri
    args: build --target ${{ matrix.target }}
```

**Add schema copy before frontend build:**
```yaml
- name: Copy schema for bundling
  shell: bash
  run: |
    mkdir -p dist/schema
    cp -r schema/* dist/schema/
```

**Update `create-release` job** to trigger on workflow_dispatch too:
```yaml
create-release:
  needs: build-tauri
  runs-on: ubuntu-latest
  steps:
    - name: Download all artifacts
      uses: actions/download-artifact@v4
      with:
        path: artifacts

    - name: List artifacts
      run: find artifacts -type f | head -50

    - name: Create GitHub Release
      uses: softprops/action-gh-release@v2
      with:
        tag_name: v0.31.20
        name: AgentMux v0.31.20
        files: |
          artifacts/**/*.exe
          artifacts/**/*.dmg
          artifacts/**/*.AppImage
          artifacts/**/*.deb
          artifacts/**/*.tar.gz
          artifacts/**/*.zip
        draft: true
        prerelease: false
        generate_release_notes: false
        body: |
          ## AgentMux v0.31.20

          First public release of AgentMux - an open-source AI-native terminal.

          ### Downloads

          | Platform | Installer | Portable |
          |----------|-----------|----------|
          | Windows x64 | `.exe` setup | `.zip` |
          | macOS ARM64 | `.dmg` | `.tar.gz` |
          | macOS x64 | `.dmg` | `.tar.gz` |
          | Linux x64 | `.deb` | `.AppImage` |
      env:
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

### Step 2: Add Linux Bundler Targets

Update `src-tauri/tauri.conf.json` bundle targets to include all platforms:

**Current:** `"targets": ["nsis"]` (Windows only)

**Change to:** `"targets": "all"` or make it platform-aware.

Since Tauri auto-detects platform, the simplest approach is:
```json
"targets": "all"
```

This produces:
- Windows: NSIS installer
- macOS: DMG + .app bundle
- Linux: Deb + AppImage

Alternatively, keep `"targets": ["nsis"]` and override per-platform in CI:
```yaml
# In the Tauri build step args:
# Linux:  --bundles deb,appimage
# macOS:  --bundles dmg
# Windows: --bundles nsis
```

### Step 3: Portable Builds

**Windows portable** is handled post-build by `scripts/package-portable.ps1`:
- Add a CI step after Tauri build (Windows only) that runs the portable script
- Or: create the zip manually from the NSIS output directory

**macOS portable** is the `.app` bundle itself:
- Tauri produces `AgentMux.app` inside the DMG
- CI can also tar the `.app` bundle: `tar -czf AgentMux-darwin-arm64.tar.gz AgentMux.app`

**Linux portable** is the AppImage:
- Tauri/AppImage is self-contained, no install needed
- Already portable by nature

### Step 4: GitHub Secrets Required

The following secrets must be configured on `agentmuxai/agentmux`:

| Secret | Purpose | Required? |
|--------|---------|-----------|
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri update signature key | Optional (for auto-updater) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for above | Optional (for auto-updater) |

`GITHUB_TOKEN` is automatically provided by Actions.

For macOS code signing (optional but recommended):
| Secret | Purpose |
|--------|---------|
| `APPLE_CERTIFICATE` | Base64-encoded .p12 certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Certificate password |
| `APPLE_SIGNING_IDENTITY` | Developer ID Application identity |
| `APPLE_ID` | Apple ID for notarization |
| `APPLE_PASSWORD` | App-specific password for notarization |
| `APPLE_TEAM_ID` | Apple Developer Team ID |

**For v0.31.20 initial release:** Skip code signing. Users can bypass Gatekeeper on macOS. Add signing in a future release.

### Step 5: Trigger the Release

```bash
# Option A: Manual workflow dispatch from GitHub UI
# Go to Actions > Build Tauri > Run workflow

# Option B: Via gh CLI
gh workflow run tauri-build.yml --repo agentmuxai/agentmux

# Option C: Tag-triggered (if workflow is updated to trigger on tags)
git tag v0.31.20
git push origin v0.31.20
```

After build completes:
1. Download artifacts from the workflow run
2. Verify each artifact works (install test on each platform)
3. Edit the draft release, finalize notes, publish

### Step 6: Post-Release

1. Upload artifacts to S3 for dl.agentmux.ai:
   ```bash
   task artifacts:upload  # stages to S3
   task artifacts:publish:v0.31.20  # promotes to releases bucket
   ```

2. Submit to package managers (future):
   ```bash
   task artifacts:winget:publish:v0.31.20
   task artifacts:snap:publish:v0.31.20
   ```

---

## Complete Rewritten Workflow

```yaml
name: Build & Release

on:
  workflow_dispatch:
    inputs:
      create_release:
        description: 'Create a GitHub release'
        required: false
        default: 'true'
        type: boolean

jobs:
  build-tauri:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            platform: linux
            arch: x64
            bundles: deb,appimage
          - os: macos-latest
            target: aarch64-apple-darwin
            platform: darwin
            arch: arm64
            bundles: dmg
          - os: macos-13
            target: x86_64-apple-darwin
            platform: darwin
            arch: x64
            bundles: dmg
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            platform: windows
            arch: x64
            bundles: nsis

    runs-on: ${{ matrix.os }}

    steps:
      - name: Checkout repository
        uses: actions/checkout@v4

      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '20'
          cache: 'npm'

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Rust cache
        uses: swatinem/rust-cache@v2
        with:
          workspaces: '. -> target'

      - name: Install Linux dependencies
        if: matrix.platform == 'linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libappindicator3-dev \
            librsvg2-dev \
            patchelf \
            libssl-dev \
            libgtk-3-dev

      - name: Install frontend dependencies
        run: npm ci

      - name: Build Rust backend binaries
        shell: bash
        run: |
          VERSION=$(node -p "require('./package.json').version")

          echo "Building agentmuxsrv-rs v${VERSION} for ${{ matrix.target }}"
          cargo build --release -p agentmuxsrv-rs --target ${{ matrix.target }}

          echo "Building wsh-rs v${VERSION} for ${{ matrix.target }}"
          cargo build --release -p wsh-rs --target ${{ matrix.target }}

          # Copy as Tauri external binaries (sidecar naming)
          mkdir -p src-tauri/binaries/bin
          if [ "${{ matrix.platform }}" = "windows" ]; then
            EXT=".exe"
          else
            EXT=""
          fi

          cp "target/${{ matrix.target }}/release/agentmuxsrv-rs${EXT}" \
             "src-tauri/binaries/agentmuxsrv-rs-${{ matrix.target }}${EXT}"
          cp "target/${{ matrix.target }}/release/wsh${EXT}" \
             "src-tauri/binaries/wsh-${{ matrix.target }}${EXT}"

          # Versioned wsh for resource bundling
          cp "target/${{ matrix.target }}/release/wsh${EXT}" \
             "src-tauri/binaries/bin/wsh-${VERSION}-${{ matrix.platform }}.${{ matrix.arch }}${EXT}"

          echo "Sidecar binaries:"
          ls -lh src-tauri/binaries/
          ls -lh src-tauri/binaries/bin/

      - name: Copy schema for bundling
        shell: bash
        run: |
          mkdir -p dist/schema
          cp -r schema/* dist/schema/

      - name: Build frontend
        run: npx vite build --mode production --config vite.config.tauri.ts

      - name: Build Tauri application
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
        with:
          tauriScript: npx tauri
          args: build --target ${{ matrix.target }} --bundles ${{ matrix.bundles }}

      - name: Create portable ZIP (Windows)
        if: matrix.platform == 'windows'
        shell: pwsh
        run: |
          $version = (Get-Content package.json | ConvertFrom-Json).version
          $portableDir = "dist/agentmux-${version}-x64-portable"
          New-Item -ItemType Directory -Force -Path $portableDir | Out-Null
          New-Item -ItemType Directory -Force -Path "$portableDir/bin" | Out-Null

          # Copy Tauri exe
          Copy-Item "src-tauri/target/${{ matrix.target }}/release/agentmux.exe" "$portableDir/"
          # Copy backend
          Copy-Item "target/${{ matrix.target }}/release/agentmuxsrv-rs.exe" "$portableDir/"
          # Copy wsh
          Copy-Item "target/${{ matrix.target }}/release/wsh.exe" "$portableDir/bin/wsh-${version}-windows.x64.exe"

          # Create ZIP
          Compress-Archive -Path "$portableDir/*" -DestinationPath "dist/AgentMux-${version}-windows-x64-portable.zip"
          Write-Host "Portable ZIP created: dist/AgentMux-${version}-windows-x64-portable.zip"

      - name: Upload installer artifacts
        uses: actions/upload-artifact@v4
        with:
          name: agentmux-${{ matrix.platform }}-${{ matrix.arch }}-installer
          path: |
            src-tauri/target/${{ matrix.target }}/release/bundle/nsis/*.exe
            src-tauri/target/${{ matrix.target }}/release/bundle/dmg/*.dmg
            src-tauri/target/${{ matrix.target }}/release/bundle/deb/*.deb
            src-tauri/target/${{ matrix.target }}/release/bundle/appimage/*.AppImage
          retention-days: 30
          if-no-files-found: ignore

      - name: Upload portable artifacts
        uses: actions/upload-artifact@v4
        with:
          name: agentmux-${{ matrix.platform }}-${{ matrix.arch }}-portable
          path: |
            dist/*portable*.zip
            src-tauri/target/${{ matrix.target }}/release/bundle/macos/*.app
          retention-days: 30
          if-no-files-found: ignore

  create-release:
    needs: build-tauri
    if: inputs.create_release == 'true' || inputs.create_release == true
    runs-on: ubuntu-latest
    permissions:
      contents: write

    steps:
      - name: Checkout repository
        uses: actions/checkout@v4

      - name: Download all artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts

      - name: List all artifacts
        run: find artifacts -type f | sort

      - name: Get version
        id: version
        run: echo "version=$(node -p \"require('./package.json').version\")" >> $GITHUB_OUTPUT

      - name: Create tag
        run: |
          git tag v${{ steps.version.outputs.version }}
          git push origin v${{ steps.version.outputs.version }}

      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          tag_name: v${{ steps.version.outputs.version }}
          name: AgentMux v${{ steps.version.outputs.version }}
          files: artifacts/**/*
          draft: true
          prerelease: false
          body: |
            ## AgentMux v${{ steps.version.outputs.version }}

            First public release of AgentMux - an open-source AI-native terminal.

            ### Downloads

            | Platform | Installer | Portable |
            |----------|-----------|----------|
            | **Windows x64** | `.exe` setup | `.zip` |
            | **macOS ARM64** (Apple Silicon) | `.dmg` | |
            | **macOS x64** (Intel) | `.dmg` | |
            | **Linux x64** | `.deb` | `.AppImage` |

            ### Installation

            **Windows:** Download and run the `.exe` installer, or extract the portable `.zip`.
            **macOS:** Download the `.dmg`, open it, drag AgentMux to Applications.
            **Linux:** Install the `.deb` package, or download the `.AppImage` and `chmod +x` it.
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

---

## Checklist

- [ ] Rewrite `.github/workflows/tauri-build.yml` (remove Go, add Rust)
- [ ] Update `src-tauri/tauri.conf.json` bundle targets if needed
- [ ] Install GitHub App permissions for Actions on `agentmuxai/agentmux`
- [ ] Configure secrets (TAURI_SIGNING keys - optional for first release)
- [ ] Push workflow to main (requires branch protection bypass or PR)
- [ ] Trigger workflow via Actions UI or `gh workflow run`
- [ ] Verify all 4 platform artifacts build successfully
- [ ] Download and smoke-test each artifact
- [ ] Edit draft release, publish
- [ ] Upload to S3 (`task artifacts:upload` + `task artifacts:publish:v0.31.20`)

---

## Risks

| Risk | Mitigation |
|------|------------|
| macOS builds unsigned | Users must right-click > Open to bypass Gatekeeper. Add signing later. |
| Linux AppImage missing deps | Tauri bundles webkit2gtk; most distros have it. Document if issues arise. |
| Rust compile time in CI | ~15-20 min per platform. Rust cache action helps on subsequent builds. |
| tauri-action version | Using `@v0` (latest v0.x). Pin to specific version if issues arise. |
| Branch protection blocks workflow push | Temporarily disable protection or use admin PAT to push |
