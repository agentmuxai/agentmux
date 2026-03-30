# CEF Portable Size Reduction Spec

**Status:** Research complete, ready for implementation decisions
**Platforms:** Windows x64, macOS (arm64 + x86_64), Linux x64
**Constraints:** GPU rendering must be preserved
**Branch:** agentx/cef-integration (v0.32.110)

---

## Per-Platform Distribution Sizes (stock CEF 146)

| Platform | Main Library | Total Runtime | Compressed (7z) |
|----------|-------------|---------------|-----------------|
| Windows x64 | `libcef.dll` 251 MB | ~365 MB | ~150 MB |
| macOS arm64 | `Chromium Embedded Framework` ~250 MB | ~300 MB | ~118 MB |
| macOS x86_64 | `Chromium Embedded Framework` ~270 MB | ~320 MB | ~123 MB |
| Linux x64 | `libcef.so` ~280 MB | ~350 MB | ~292 MB |
| Linux arm64 | `libcef.so` ~320 MB | ~430 MB | ~365 MB |

The main library is ~70% of every distribution. File-level trimming is marginal.

**IMPORTANT:** The official CEF builds (hosted on Spotify CDN, the official CEF build infra)
already use `is_official_build=true` with full optimization (`/OPT:REF`, `/OPT:ICF`, LTO).
PDB files are excluded from the minimal distribution. The 251 MB libcef.dll is **already
stripped and optimized** — `symbol_level=0` would yield single-digit percent savings at best,
not the ~50% initially estimated. The size is inherent to Chromium's codebase.

---

## Per-Platform File Inventory

### Windows x64

| File | Size | Required | GPU? |
|------|------|----------|------|
| `libcef.dll` | 251 MB | YES | Contains GPU code |
| `chrome_elf.dll` | 2.4 MB | YES | |
| `icudtl.dat` | ~10 MB | YES | |
| `v8_context_snapshot.bin` | ~1 MB | YES | |
| `libEGL.dll` | ~0.5 MB | **YES (GPU)** | ANGLE OpenGL ES |
| `libGLESv2.dll` | ~8 MB | **YES (GPU)** | ANGLE GLES2 |
| `d3dcompiler_47.dll` | 4.6 MB | **YES (GPU)** | WebGL + CSS 3D |
| `chrome_100_percent.pak` | ~1-2 MB | YES | |
| `chrome_200_percent.pak` | ~2 MB | KEEP (HiDPI) | |
| `resources.pak` | ~10 MB | YES | |
| `locales/en-US.pak` | ~400 KB | YES | |
| `locales/*.pak` (49 others) | ~18 MB | NO | |
| `vk_swiftshader.dll` | 5.2 MB | NO | Software fallback |
| `vk_swiftshader_icd.json` | 1 KB | NO | |
| `vulkan-1.dll` | 915 KB | NO | |
| `dxil.dll` | ~1.5 MB | NO | WebGPU only |
| `dxcompiler.dll` | ~5 MB | NO | WebGPU only |

### macOS (arm64 / x86_64)

```
MyApp.app/Contents/
  MacOS/agentmux-cef
  Frameworks/
    Chromium Embedded Framework.framework/
      Chromium Embedded Framework          ~250-270 MB (the monolith)
      Libraries/
        libEGL.dylib                       ~0.5 MB     ** KEEP (GPU) **
        libGLESv2.dylib                    ~8 MB       ** KEEP (GPU) **
        libswiftshader_libEGL.dylib        ~0.5 MB     REMOVE
        libswiftshader_libGLESv2.dylib     ~3 MB       REMOVE
        libvk_swiftshader.dylib            ~6 MB       REMOVE
        vk_swiftshader_icd.json            ~1 KB       REMOVE
        libcef_sandbox.a                   varies      optional
      Resources/
        icudtl.dat                         ~10 MB
        v8_context_snapshot.arm64.bin       ~1 MB       arch-specific
        v8_context_snapshot.x86_64.bin      ~1 MB       arch-specific
        chrome_100_percent.pak             ~1-2 MB
        chrome_200_percent.pak             ~2 MB        KEEP (Retina)
        resources.pak                      ~10 MB
        locales/en-US.pak                  ~400 KB
        locales/*.pak                      ~18 MB       REMOVE
    MyApp Helper.app/                                   renderer process
    MyApp Helper (Alerts).app/                          alert subprocess
    MyApp Helper (GPU).app/                             ** KEEP (GPU) **
    MyApp Helper (Plugin).app/                          plugin subprocess
    MyApp Helper (Renderer).app/                        renderer subprocess
```

**macOS-specific requirements:**
- Code signing innermost-first: Libraries -> Framework -> Helpers -> Main app
- Hardened runtime: `codesign --options runtime --timestamp`
- Notarization required for distribution outside App Store
- V8 snapshot is arch-specific (`v8_context_snapshot.arm64.bin` vs `.x86_64.bin`)
- CEF does NOT ship universal binaries; use `lipo` to merge if needed
- `chrome_200_percent.pak` should be KEPT on macOS (Retina is standard)

### Linux x64

| File | Size | Required | GPU? |
|------|------|----------|------|
| `libcef.so` | ~280 MB | YES | Contains GPU code |
| `chrome-sandbox` | ~20 KB | YES (SUID) | |
| `icudtl.dat` | ~10 MB | YES | |
| `v8_context_snapshot.bin` | ~0.7 MB | YES | |
| `libEGL.so` | ~0.5 MB | **YES (GPU)** | ANGLE |
| `libGLESv2.so` | ~7 MB | **YES (GPU)** | ANGLE |
| `chrome_100_percent.pak` | ~0.4 MB | YES | |
| `chrome_200_percent.pak` | ~0.5 MB | KEEP (HiDPI) | |
| `resources.pak` | ~5 MB | YES | |
| `locales/en-US.pak` | ~400 KB | YES | |
| `locales/*.pak` (49 others) | ~5 MB | NO | |
| `libvk_swiftshader.so` | ~6 MB | NO | Software fallback |
| `libvulkan.so.1` | ~0.2 MB | NO | |
| `vk_swiftshader_icd.json` | ~0.2 KB | NO | |

**Linux-specific requirements:**
- `chrome-sandbox` needs SUID: `chown root:root && chmod 4755` (or use `--no-sandbox`)
- Link with `-Wl,-rpath,$ORIGIN` so `libcef.so` is found adjacent to executable
- Runtime deps: `libgtk-3-0 libgbm1 libnss3 libxss1 libasound2 libatk1.0-0 libatk-bridge2.0-0 libdrm2 libxcomposite1 libxdamage1 libxrandr2 libpango-1.0-0 libcairo2 libcups2 libdbus-1-3 libexpat1 libfontconfig1 libnspr4 libxfixes3 libxkbcommon0`
- glibc 2.29+ (Ubuntu 20.04+, Debian 11+)
- Wayland: `--ozone-platform=wayland` (X11 is default)

---

## GPU Rendering — What to Keep

### GPU Fallback Chain (Chromium)

```
1. HARDWARE_VULKAN  →  GPU with Vulkan + OpenGL
2. HARDWARE_GL      →  GPU with OpenGL/Metal only
3. SWIFTSHADER      →  Software WebGL (CPU-based) ← we are removing this
4. DISPLAY_COMPOSITOR → compositing only, no WebGL
```

Without SwiftShader, machines with no GPU get DISPLAY_COMPOSITOR (2D works, WebGL fails).
This is acceptable for AgentMux — it's a desktop app, always has a GPU.

### Files to KEEP for GPU (per platform)

| Platform | Required GPU Files |
|----------|-------------------|
| Windows | `libEGL.dll`, `libGLESv2.dll`, `d3dcompiler_47.dll` |
| macOS | `Libraries/libEGL.dylib`, `Libraries/libGLESv2.dylib` |
| Linux | `libEGL.so`, `libGLESv2.so` |

### Files SAFE to Remove (all platforms)

| Category | Windows | macOS | Linux |
|----------|---------|-------|-------|
| SwiftShader | `vk_swiftshader.dll`, `vulkan-1.dll`, `vk_swiftshader_icd.json` | `libvk_swiftshader.dylib`, `libswiftshader_libEGL.dylib`, `libswiftshader_libGLESv2.dylib`, `vk_swiftshader_icd.json` | `libvk_swiftshader.so`, `libvulkan.so.1`, `vk_swiftshader_icd.json` |
| WebGPU | `dxil.dll`, `dxcompiler.dll` | N/A | N/A |
| Locales | 49 locale paks | 49 locale paks | 49 locale paks |

**SwiftShader is being deprecated upstream** (Chrome 137+): already disabled on macOS/Linux,
replaced by WARP on Windows. Safe to drop now.

---

## Reduction Scenarios (cross-platform)

### Scenario A: File Stripping Only (no rebuild)

| Action | Win x64 | macOS arm64 | Linux x64 |
|--------|---------|-------------|-----------|
| Strip locales (keep en-US) | -18 MB | -18 MB | -5 MB |
| Remove SwiftShader | -6 MB | -10 MB | -6 MB |
| Remove WebGPU DLLs | -6.5 MB | N/A | N/A |
| **Savings** | **~30 MB** | **~28 MB** | **~11 MB** |
| **Result (disk)** | **~335 MB** | **~272 MB** | **~339 MB** |

Effort: trivial. Marginal gain since the main library dominates.

### Scenario B: Custom Build — Feature Disable

**NOTE:** The official CEF builds already use `is_official_build=true` with full
optimization and ship without PDB files. `symbol_level=0` does NOT halve the DLL —
the 251 MB is already optimized. The only meaningful savings from a custom build
come from **disabling features**.

```gn
# Already set by official builds (included for completeness)
is_official_build=true
use_thin_lto=true
exclude_unwind_tables=true

# Disable features we don't need (safe for AgentMux)
enable_basic_printing=false       # ~5-10 MB
enable_print_preview=false
enable_pdf=false
enable_tagged_pdf=false
enable_spellcheck=false           # ~1-3 MB
proprietary_codecs=false          # ~5-10 MB
ffmpeg_branding="Chromium"
enable_av1_decoder=false          # ~2-5 MB
enable_dav1d_decoder=false
enable_hls_demuxer=false
enable_extensions=false           # ~2-5 MB
enable_vr=false                   # ~1-2 MB
enable_widevine=false             # ~1-2 MB
enable_nacl=false
enable_profiling=false
optimize_webui=true

# KEEP GPU support
# enable_vulkan — leave default (enabled)
enable_swiftshader=false          # ~3-5 MB (deprecated upstream anyway)
enable_swiftshader_vulkan=false
```

Estimated savings from feature disables: **~20-40 MB** from the main library.

| Platform | Before (stock) | After (estimated) | Download (7z) |
|----------|---------------|-------------------|---------------|
| Windows x64 | 365 MB | ~295-315 MB | ~120-130 MB |
| macOS arm64 | 300 MB | ~240-260 MB | ~95-105 MB |
| Linux x64 | 350 MB | ~280-300 MB | ~140-155 MB |

Honest assessment: custom builds save ~20-40 MB on disk per platform. The effort
(8+ hour builds, 150 GB disk, CI pipeline) may not justify the savings.

### Scenario C: Just Compress for Distribution (no rebuild)

| Platform | ZIP | 7z (LZMA2) |
|----------|-----|-----------|
| Windows x64 | ~150 MB | ~120 MB |
| macOS arm64 | ~120 MB | ~95 MB |
| Linux x64 | ~200 MB | ~165 MB |

---

## Feature Impact Matrix

| Feature | GN Arg | Est. Savings | AgentMux Impact |
|---------|--------|-------------|-----------------|
| PDF viewer | `enable_pdf=false` | 5-10 MB | None |
| Print | `enable_basic_printing=false` | 5-10 MB | None |
| Spellcheck | `enable_spellcheck=false` | 1-3 MB | None |
| Proprietary codecs | `proprietary_codecs=false` | 5-10 MB | None |
| AV1 decoders | `enable_av1_decoder=false` | 2-5 MB | None |
| VR/WebXR | `enable_vr=false` | 1-2 MB | None |
| Widevine DRM | `enable_widevine=false` | 1-2 MB | None |
| Extensions | `enable_extensions=false` | 2-5 MB | None |
| SwiftShader | `enable_swiftshader=false` | 3-5 MB | None (deprecated upstream) |
| FFmpeg entirely | `media_use_ffmpeg=false` | 10-20 MB | **Breaks ALL audio/video** |
| WebRTC | N/A | 0 | **Cannot disable at compile time** |
| GPU/ANGLE | N/A | — | **MUST KEEP** |

---

## Important Caveats

1. **GPU files are non-negotiable.** `libEGL`, `libGLESv2`, `d3dcompiler_47.dll` (Windows) must ship. Without them, rendering falls back to CPU compositing with no WebGL.

2. **SwiftShader is being removed upstream.** Chrome 137+ disables it on macOS/Linux; Windows replaces it with WARP. CEF 146 tracks Chromium 146, so this is already in play. Safe to drop now.

3. **WebRTC has no compile-time disable.** No `declare_args()` block exists. Runtime-only mitigation via preferences.

4. **UPX is not viable.** CFGuard incompatibility, sandbox crashes, unstable x64 PE support.

5. **CEF version lock.** Our `cef` Rust crate pins to CEF 146. Custom build must match this exact branch.

6. **macOS code signing order matters.** Sign innermost-first: Libraries -> Framework -> Helpers -> Main app. Hardened runtime + notarization required.

7. **Linux sandbox requires SUID.** `chrome-sandbox` needs `chown root:root && chmod 4755`, or ship with `--no-sandbox` and document the trade-off.

---

## Cross-Platform Build Pipeline

### Build Requirements

| Platform | Toolchain | Disk Space | RAM | Build Time |
|----------|-----------|-----------|-----|-----------|
| Windows x64 | VS 2022+, Windows SDK | 150-200 GB | 16+ GB | ~6 hours |
| macOS arm64 | Xcode 15+, macOS 14+ SDK | 100-150 GB | 16+ GB | 3-5 hours |
| macOS x86_64 | Xcode 15+ | 100-150 GB | 16+ GB | 6+ hours |
| Linux x64 | Clang (auto), sysroot (auto) | 100-150 GB | 16+ GB | 2-4 hours |

### CI Considerations

Standard GitHub Actions runners **will not work** for building CEF from source:
- 14 GB disk (need 150+ GB)
- macOS Intel runners hit 6-hour timeout at ~75%
- Chromium checkout alone is ~40 GB

**Viable options:**
1. **Self-hosted runners** — 200+ GB SSD, 32+ GB RAM, 16+ cores (best)
2. **Pre-built CEF from Spotify CDN** — download stock binaries, strip files only (easiest)
3. **One-time local builds** — build on dev machines, upload artifacts to GitHub Releases
4. **Large GH Actions runners** — paid tier with 200 GB disk (expensive)

### Build Script (all platforms)

```bash
# Clone depot_tools
git clone https://chromium.googlesource.com/chromium/tools/depot_tools.git
export PATH="$(pwd)/depot_tools:$PATH"

# GN args (same across platforms)
export GN_DEFINES="is_official_build=true \
  symbol_level=0 blink_symbol_level=0 v8_symbol_level=0 \
  use_thin_lto=true exclude_unwind_tables=true \
  enable_pdf=false enable_basic_printing=false enable_print_preview=false \
  enable_spellcheck=false proprietary_codecs=false ffmpeg_branding=Chromium \
  enable_av1_decoder=false enable_dav1d_decoder=false enable_hls_demuxer=false \
  enable_extensions=false enable_vr=false enable_widevine=false \
  enable_swiftshader=false enable_swiftshader_vulkan=false \
  optimize_webui=true enable_profiling=false"

# Build
python3 automate-git.py \
  --download-dir=/path/to/chromium \
  --branch=7680 \
  --minimal-distrib \
  --client-distrib \
  --force-clean \
  --x64-build \               # or --arm64-build for macOS arm64
  --no-debug-build
```

---

## Recommended Plan

### Phase 1: File Stripping (immediate)

Add to portable packaging script per platform:
- Strip all locales except `en-US.pak`
- Remove SwiftShader files
- Remove WebGPU DLLs (Windows only)
- Keep all GPU files (`libEGL`, `libGLESv2`, `d3dcompiler_47`)
- Keep `chrome_200_percent.pak` (HiDPI/Retina)

**Result: ~5-30 MB savings per platform (marginal but free)**

### Phase 2: Distribution Compression

- Use 7z/LZMA2 for download packages
- Platform-specific: `.tar.xz` on Linux, `.dmg` on macOS, `.7z` on Windows

**Result: 40-60% download size reduction**

### Phase 3: Custom CEF Build (diminishing returns)

- One-time local builds for each platform (8+ hrs, 150+ GB disk each)
- Upload to GitHub Releases as pre-built artifacts
- Update `cef` Rust crate to download from our releases instead of Spotify CDN
- Saves ~20-40 MB per platform — may not justify the build infra investment

**Result: ~280-315 MB disk per platform, ~120-155 MB download**

### Phase 4: macOS Distribution

- Implement `.app` bundle with helper apps (GPU, Renderer, Alerts, Plugin)
- Code sign + notarize pipeline
- Consider universal binary (arm64 + x86_64) via `lipo` — doubles CEF size but single download

### Phase 5: Linux Distribution

- AppImage or tarball with `chrome-sandbox` + SUID instructions
- Ship `LD_LIBRARY_PATH` launcher script or set rpath at link time
- Document system dependencies
- Wayland support via `--ozone-platform=wayland`

---

## Size Summary (corrected)

| Approach | Win x64 | macOS arm64 | Linux x64 |
|----------|---------|-------------|-----------|
| **Stock CEF 146** | 365 MB / 150 MB dl | 300 MB / 118 MB dl | 350 MB / 292 MB dl |
| File strip only | 335 MB / 135 MB dl | 272 MB / 95 MB dl | 339 MB / 165 MB dl |
| Custom build (features) | 295-315 MB / 120-130 MB dl | 240-260 MB / 95-105 MB dl | 280-300 MB / 140-155 MB dl |
| **Tauri (WebView2)** | **2-10 MB** | N/A (WebKit) | N/A (WebKit) |

**Reality check:** The main library (libcef.dll/so) is already fully optimized in stock builds.
Custom builds save ~20-40 MB by disabling features, not by stripping symbols.
The CEF trade-off is ~100-150 MB download for full Chromium control vs ~5 MB for system WebView.
Both builds ship in parallel — CEF is the "works everywhere with identical behavior" option.

---

## Sources

- CEF README.redistrib.txt — official required/optional file list
- CEF cef_variables.cmake.in — per-platform binary/resource file lists
- CEF gn_args.py — official GN args reference
- CEF Automated Builds — cef-builds.spotifycdn.com
- Chromium GPU fallback docs — content/browser/gpu/fallback.md
- Chrome Status: SwiftShader removal — chromestatus.com/feature/5166674414927872
- CEF Forum threads on size reduction, macOS notarization, CI builds
- CefSharp Output Files Description Table
- Thorium GN Arguments Documentation
