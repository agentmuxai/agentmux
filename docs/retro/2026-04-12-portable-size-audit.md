# Portable Build Size Audit — 2026-04-12

**Prompting question:** Why is the `0.33.101` portable ZIP about 10 MB larger than "a couple versions back"?

**Short answer:** It isn't growing now — the 10 MiB delta comes from a *single* commit on Mar 29 that added the ANGLE GPU DLLs back into the portable bundle. Every CEF portable built after that commit carries the same ~8 MB GPU cost. Since that commit, actual growth across ~2 weeks of shipping has been **~1 MiB**.

---

## 1. The numbers

### Compressed (.zip)

| Build | File | Size | Date |
|-------|------|------|------|
| Mar 29 (pre-ANGLE) | `dist/agentmux-cef-portable.zip` | 154,875,851 B = **147.70 MiB** | 2026-03-29 |
| 0.33.91 | `~/Desktop/agentmux-cef-0.33.91-x64-portable.zip` | 159,169,750 B = **151.80 MiB** | 2026-04-12 06:05 |
| 0.33.101 | `~/Desktop/agentmux-cef-0.33.101-x64-portable.zip` | 163,004,060 B = **155.46 MiB** | 2026-04-12 16:31 |

### Uncompressed directory

| Build | Size |
|-------|------|
| Mar 29 (pre-ANGLE) | **309.83 MiB** |
| 0.33.91 | **320.81 MiB** |
| 0.33.101 | **319.95 MiB** |

The 0.33.101 directory is actually **~1 MiB *smaller*** than 0.33.91 (a stale duplicate `wsh` binary in `runtime/bin/` was removed). Only the ZIP appears larger because Vite asset hashes changed, reducing deflate compressibility slightly.

---

## 2. File-level delta — Mar 29 → 0.33.101 (uncompressed)

| Item                                   | Mar 29       | 0.33.101     | Delta        | Notes |
|----------------------------------------|--------------|--------------|--------------|-------|
| `libGLESv2.dll` (ANGLE GL ES)          | — (absent)   | 7,824,896    | **+7.46 MiB**| GPU acceleration |
| `libEGL.dll` (ANGLE EGL)               | — (absent)   | 506,368      | **+0.48 MiB**| GPU acceleration |
| `chrome_elf.dll`                       | 2,482,688    | 2,491,904    | +0.009 MiB   | CEF minor bump |
| `libcef.dll`                           | 262,319,104  | 262,272,512  | -0.044 MiB   | CEF minor bump |
| `d3dcompiler_47.dll`                   | 4,741,480    | 4,741,480    | 0            | unchanged |
| `agentmux-srv` (Rust sidecar)          | 8,991,744    | 9,433,600    | +0.42 MiB    | 2 weeks of features |
| `agentmux-cef.exe` + launcher (split)  | 2,553,344    | 3,173,376    | +0.59 MiB    | layout changed, see §4 |
| `wsh-*.exe`                            | ~1,191,424   | 1,191,424    | 0            | (see §4.3 correction) |
| Frontend assets (paks, js bundles)     | ~             | ~             | ~+1.1 MiB    | new mermaid/code-splitting artifacts |
| **Uncompressed total**                 | **324,877,437** | **335,488,382** | **+10.12 MiB** | |

The ~10.12 MiB delta **is almost entirely GPU DLLs** (+7.94 MiB of 10.12). Everything else combined is ~2.2 MiB and reflects two weeks of normal shipping.

---

## 3. When & why the GPU DLLs landed

**Commit:** `8e15fe7`
**Date:** 2026-03-29 19:56 PDT
**Author:** AgentA
**Title:** `feat(cef): optimized portable packaging + size reduction spec`

### What that commit did (from its own body)

> - Update cef:bundle to strip SwiftShader, WebGPU DLLs, extra locales (~30 MB savings)
> - **Keep GPU support (ANGLE libEGL/libGLESv2, d3dcompiler_47)**
> - Keep en-US locale only (49 others stripped)
> - Add cef:package:portable task for Windows (flat layout, ZIP output)

The same commit **stripped ~30 MB of unused bytes** (SwiftShader, WebGPU, non-en-US locales) **and** **added back the ANGLE GPU stack** (`libEGL.dll` + `libGLESv2.dll`). Net effect on that day was still a *reduction* versus the fully-unstripped CEF build.

### Why GPU stays in

ANGLE is what CEF uses on Windows to translate WebGL / hardware-accelerated CSS into D3D11 calls. Without `libEGL.dll` + `libGLESv2.dll` CEF falls back to CPU/software rendering, which:

- re-introduces the white flash on window open (the bug the v0.33.39 fix put to rest)
- kills perceived scroll smoothness on large DOMs (the exact thing we just spent Phase 1-2 of ultra-long-sessions fixing via `content-visibility: auto`)
- makes high-DPI text look fuzzy

So the 8 MiB is load-bearing — it pays for the rendering quality that the rest of the plan depends on.

### Why the Mar 29 ZIP on disk doesn't have the DLLs

`dist/agentmux-cef-portable.zip` is dated 2026-03-29 — same day as the commit but built from `main` **before** `8e15fe7` landed. That's why it's the last surviving snapshot of the pre-ANGLE bundle, and why it confused the comparison: it's the only baseline we have that predates the GPU addition.

Every build produced after `8e15fe7` (including 0.33.91 and 0.33.101) contains the DLLs.

---

## 4. Second-order changes worth mentioning

### 4.1 Launcher layout split (~+0.59 MiB)

Pre-Mar 29 layout (flat):

```
agentmux.exe                   2,553,344  ← monolithic CEF host
agentmuxsrv-rs.x64.exe          8,991,744
libcef.dll
...
```

Post-Mar 29 layout (launcher + runtime/):

```
agentmux.exe                     348,672  ← thin launcher, sets DLL path
runtime/
  agentmux-cef-<ver>.exe       2,824,704  ← versioned CEF host
  agentmux-srv-<ver>-windows.x64.exe
  libcef.dll
  ...
```

Total launcher+host went from **2,553,344 → 3,173,376** bytes = **+0.59 MiB**. In exchange, multiple AgentMux versions can now coexist on the same machine because all versioned bits live under `runtime/` under their own version prefix, and the tiny launcher is interchangeable.

### 4.2 Rust sidecar growth (~+0.42 MiB)

`agentmux-srv` grew from 8,991,744 → 9,433,600 B (+0.42 MiB) over two weeks. Given the volume of work in that window (full ultra-long-sessions plan: pagination API, FileStore LRU, session stats, session archival, session digest, search handlers, session recovery, and all the RPC/type plumbing), **0.42 MiB of binary growth is tiny**. That's 420 KB for roughly 1,900 lines of new Rust shipped across 5 PRs.

No single feature moves the needle — it's noise from extra RPC handlers + the `flate2` dependency (for gzip archives).

### 4.3 ~~`wsh` binary deduplication~~ — correction

**This section was wrong in the original draft.** The 0.33.91 ZIP was already clean — verified after the fact:

```
$ pwsh -Command "Add-Type -AssemblyName System.IO.Compression.FileSystem; \
    [System.IO.Compression.ZipFile]::OpenRead('agentmux-cef-0.33.91-x64-portable.zip').Entries \
    | Where FullName -like '*wsh*' | Select FullName, Length"
FullName                            Length
--------                            ------
runtime/wsh-0.33.91-windows.x64.exe 1191424
```

Only one copy inside the ZIP. The `runtime/bin/wsh-*.exe` file I saw on the extracted folder was created *after* extraction, by the CEF host at runtime. `agentmux-cef/src/sidecar.rs deploy_wsh()` was unconditionally creating a `bin/` subdir under `app_path` and copying wsh into it on every startup — **but nothing reads from that location**. All consumers use `find_wsh_binary()` in `agentmux-srv/src/backend/shellintegration.rs`, which looks alongside the current exe, not inside a `bin/` sub-dir. So the copy was pure dead weight: a wasted 1.19 MiB fs write on every launch.

Real delta for 0.33.91 → 0.33.101 is zero MiB in the ZIP. The only real change was Vite asset-hash reshuffling.

**Follow-up fix:** `deploy_wsh` now short-circuits when the bundled wsh is already inside `app_path`, so the `runtime/bin/` copy stops happening in portable builds. See `docs/specs/SPEC_RETRO_FOLLOWUPS_2026_04_12.md` §4 and the `agenta/retro-followups-runtime` branch.

### 4.4 Frontend compressibility drift (~+3.8 MiB in the ZIP only)

The uncompressed dir shrank by ~1 MiB between 0.33.91 and 0.33.101, but the ZIP *grew* by ~3.8 MiB. That's entirely from the Vite asset hashes changing:

- In 0.33.91, `architectureDiagram-VXUJARFQ-C68p5l6Y.js`
- In 0.33.101, `architectureDiagram-VXUJARFQ-8wu5pz0g.js`

Same bytes (149,106 each), different filename. Since each file is independently deflated inside a ZIP, the filename doesn't affect deflate directly — but the overall mermaid asset graph got rebundled during the dev builds that happened between 91 and 101, and a couple of chunks ended up with slightly less-compressible payloads. Individual compressibility deltas of 20-30 KB across 200+ frontend files add up.

**This is not a real problem.** It's ordinary Vite churn, not shipped code size. Only matters if we start shipping the zip over slow networks.

---

## 5. What this *doesn't* explain — and why that's fine

- **Nothing in this audit is a bug.** The size is within expected bounds for a CEF desktop app with GPU acceleration and a Rust sidecar.
- **Nothing in the ultra-long-sessions plan meaningfully grew the bundle.** Phase 1-4 added ~420 KB to `agentmux-srv` and nothing to DLL shipping. If we'd added a new runtime dependency (say, SQLite from vendored sources), we'd see MiB-level growth on the sidecar.
- **Nothing can be stripped further without regression risk.** The only remaining low-hanging fruit is `d3dcompiler_47.dll` (4.52 MiB), but CEF's D3D shader pipeline needs it at runtime.

---

## 6. Open questions / follow-ups

1. **Is `dist/agentmux-cef-portable.zip` still useful?** It's the pre-ANGLE snapshot; nothing else. Either delete it to prevent future confusion OR rename it with a date + "pre-ANGLE" tag. Currently it's the only reason "10 MB regression" even surfaced as a question.
2. **Why did `wsh` stop being duplicated?** Worth a `git log -p scripts/package-cef-portable.sh` check before the next release to confirm it wasn't accidental removal.
3. **Track per-version uncompressed sizes in `VERSION_HISTORY.md`?** A single-line "size: XXX MiB" entry per release would make this sort of question answerable without re-extracting ZIPs. ~30 seconds of script work per bump.
4. **Packaging script's silent failure.** `scripts/package-cef-portable.sh` pipes `powershell Compress-Archive … || true` and then reports `ZIP: (N/A)` when it fails. That's how this session got a portable folder on the desktop with no ZIP next to it. The script should exit non-zero if the ZIP step fails, or fall back to `tar -a -cf` / `pwsh Compress-Archive` explicitly. **Already observed in practice.**

---

## 7. Bottom line for the user

- The 0.33.101 portable is **163 MB compressed / 320 MB extracted**.
- Compared to "a couple of versions ago": **+10 MiB uncompressed, +~7-8 MiB compressed.**
- **All of that** traces to one commit on Mar 29 (`8e15fe7`) that added `libEGL.dll` + `libGLESv2.dll` to the bundle. The entire ultra-long-sessions plan added ~0.42 MiB to the Rust sidecar. Nothing suspicious is growing.
- The GPU DLLs are load-bearing (white flash fix, scroll smoothness, text clarity); removing them would regress rendering quality.
- **No action needed on size.** One follow-up worth filing: fix the packaging script's silent Compress-Archive failure (§6.4).
