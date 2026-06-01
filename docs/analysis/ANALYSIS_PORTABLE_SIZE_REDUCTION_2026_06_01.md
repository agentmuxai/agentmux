# Portable Build Size Analysis — v0.41.0
**Date:** 2026-06-01  
**Build:** agentmux-0.41.0+g1f9447e1.20260601T094812-x64-portable.zip  
**ZIP size:** 171 MB | **Extracted:** ~410 MB

---

## Size Breakdown

| Category | Size | % of ZIP |
|---|---|---|
| **CEF runtime** (libcef.dll) | 250 MB | 61% |
| CEF resources (resources.pak, icudtl.dat, .pak files) | 30 MB | 7% |
| CEF GPU DLLs (libGLESv2, d3dcompiler_47, libEGL, chrome_elf) | 15 MB | 4% |
| **Frontend source maps** (.js.map) | 28 MB | 7% |
| Frontend JS + CSS + fonts | 17 MB | 4% |
| Rust binaries (srv, host, launcher, tools) | 30 MB | 7% |
| Misc (schema, README, etc.) | 10 MB | 2% |

### Top 15 files by size

| Size | File |
|---|---|
| 250 MB | libcef.dll |
| 18 MB | resources.pak |
| 13 MB | agentmux-srv-0.41.0-windows.x64.exe |
| 12 MB | frontend/assets/index-\*.js.map |
| 10 MB | icudtl.dat |
| 7.5 MB | libGLESv2.dll |
| 6.9 MB | agentmux-0.41.0.exe (host) |
| 5.2 MB | tools/bin/rg.exe |
| 4.5 MB | d3dcompiler_47.dll |
| 3.8 MB | agentmux.exe (launcher) |
| 3.7 MB | tools/bin/agentmux-bashwrap.exe |
| 2.9 MB | frontend/assets/index-\*.js |
| 2.4 MB | chrome_elf.dll |
| 1.8 MB | cytoscape.esm-\*.js.map |
| 1.75 MB | treemap-\*.js.map |

---

## Current optimizations already applied

| Optimization | Impact |
|---|---|
| `strip = true` in Cargo release profile | Symbols stripped from all Rust binaries |
| `lto = true` | Full cross-crate LTO |
| `opt-level = "s"` | Size-over-speed codegen |
| `codegen-units = 1` | Max cross-module optimization |
| Static CRT (`+crt-static`) | No VCRUNTIME dependency |
| en-US locale only | ~18 MB saved vs all locales |
| SwiftShader / WebGPU DLLs excluded | ~28 MB saved |
| KaTeX legacy font formats stripped | ~876 KB saved |

---

## Slimming opportunities

### Tier 1 — High impact (28–50 MB total)

#### 1. Strip dependency source maps from the portable (est. ~20 MB)
**Current:** `sourcemap: true` in vite.config.ts includes .map files for all vendor chunks (cytoscape, mermaid, KaTeX, shiki, treemap, etc.). These are 16+ MB of the 28 MB source-map total.  
**Fix:** Vite plugin or rollup hook to delete `node_modules/**` source maps post-build while keeping the app's own index-*.js.map.  
**Trade-off:** Library error stacks won't map to original source — not material since we control the app code, not the libs.  
**Effort:** Low (one Vite plugin or a `find dist -name '*.map' -path '*/node_modules/*' -delete` post-step).

#### 2. Make source maps dev-only in production portables (est. ~28 MB)
**Current:** Source maps always included because `sourcemap: true` is unconditional.  
**Context:** The runtime source-map resolver (SPEC_FE_SOURCE_MAP_RESOLVER_2026_05_27.md) uses maps to rewrite crash stacks. Maps are the "why" for keeping them.  
**Fix (shipped v0.41.1):** `task package:release` runs `find dist/frontend -name "*.map" -delete` after the Vite build. `task package` (dev portables) keeps maps. See `docs/specs/SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md`.  
**Trade-off:** Production crash stacks show minified names. Acceptable since dev builds retain full maps.  
**Effort:** Done.

#### 3. Lazy-load heavy vendor chunks (est. ~3–5 MB parsed JS)
**Current:** All vendor code in one bundle (index-\*.js = 2.9 MB). Mermaid, Cytoscape, KaTeX are loaded eagerly even on panes that never use them.  
**Fix:** Dynamic `import()` at the call site for each heavy lib. Vite splits them into separate chunks automatically.  
**Trade-off:** First use of a diagram/graph pane has a ~100ms chunk-fetch. Preload hints mitigate this.  
**Effort:** Medium — find each heavy lib's entry point, wrap in dynamic import.

---

### Tier 2 — Moderate impact (3–8 MB)

#### 4. Split agentmux-srv into feature-gated binaries (est. 3–5 MB)
**Current:** agentmux-srv = 13 MB, includes all providers, all transport layers.  
**Fix:** Cargo feature flags to build a "slim" srv that omits e.g. the heavy AI-provider clients for release builds that don't need them on Windows.  
**Effort:** High — needs feature audit of srv's dependencies.

#### 5. Replace rg.exe with a smaller Rust grep (est. ~3–4 MB)
**Current:** tools/bin/rg.exe = 5.2 MB. ripgrep bundles PCRE2, simd, and all platform targets.  
**Fix:** Build a stripped rg binary with only the options actually used (no PCRE2, no multiline, UTF-8 only). Or replace with a purpose-built `agentmux-grep` crate.  
**Effort:** Medium.

#### 6. Pack icudtl.dat more aggressively (est. 2–3 MB)
**Current:** icudtl.dat = 10 MB. This is the Unicode character database — required by Blink for text layout.  
**Fix:** CEF/Chromium supports a "small ICU" build that trims the database to just the code-point tables Chromium actually reads. Requires building CEF from source with `icu_use_data_file=false` or using Chromium's `icudt_subset`.  
**Effort:** High (custom CEF build).

#### 7. Remove chrome_200_percent.pak (est. 1.2 MB)
**Current:** chrome_200_percent.pak = 1.2 MB. Contains HiDPI UI resources (icons, cursors) for Chromium's own UI chrome — not used since CEF replaces Chromium's UI with the app's own frontend.  
**Fix:** Verify these resources are unused (no `--force-device-scale-factor` code path reads them), then exclude in `bundle:windows`.  
**Effort:** Low — one exclusion line in Taskfile.yml after verification.

---

### Tier 3 — Marginal (< 1 MB each)

| Opportunity | Est. savings | Effort |
|---|---|---|
| Subset Hack Nerd Mono fonts to glyphs actually used | 0.5–1 MB | Medium (pyftsubset) |
| Remove `v8_context_snapshot.bin` if startup time is acceptable without it | 680 KB | Low (test startup perf) |
| Minify schema JSON files | 100–200 KB | Trivial |

---

## Realistic reduction ceiling

| Scenario | Est. ZIP size | Reduction |
|---|---|---|
| Current (v0.41.0) | 171 MB | baseline |
| Strip dependency .map files only (#1) | ~155 MB | −16 MB |
| Strip all source maps in release (#2) | ~140 MB | −31 MB |
| + lazy vendor chunks (#3) | ~138 MB | −33 MB |
| + rg.exe (#5) + chrome_200% (#7) | ~133 MB | −38 MB |
| **Practical floor without CEF changes** | **~125–130 MB** | **~25–27%** |
| Extract CEF to separate download | ~10 MB stub | −94% |

---

## The unmovable cost

**libcef.dll = 250 MB extracted / ~65 MB compressed.** This is the Chromium binary — V8, Blink, networking, GPU pipeline. It's already stripped (no debug symbols), already locale-trimmed, already missing deprecated fallbacks. The only levers that move it are:

1. **Build CEF from source** with a stripped Chromium (Chromium team does this for the official Chrome installer; would require ongoing CEF fork).
2. **Separate CEF download** — ship a launcher stub that downloads CEF on first run (high engineering effort; breaks air-gap installs).
3. **Different embedded browser** — not a realistic option at this stage.

---

## Recommended order of attack

1. **Strip dependency source maps** — low effort, ~16 MB, no user-visible trade-off. Add a post-build step.
2. **Source-map resolver fallback** — spec the graceful-degrade path, then gate source maps behind a build flag. ~28 MB.
3. **lazy vendor imports** — ongoing, do as each heavy pane is touched.
4. **chrome_200_percent.pak** — verify then exclude.
5. **v8_context_snapshot.bin** — measure startup delta; remove if < 300ms regression.
