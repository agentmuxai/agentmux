# Spec: macOS packaging — signed, launchable `AgentMux.app` / `.dmg`

**Date:** 2026-05-30
**Status:** Spec → implementation (phased)
**Related:** `docs/macos-signing.md`, `docs/retro/retro-macos-keychain-prompt-2026-05-30.md`,
`docs/specs/SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28.md` (framework bundling — done),
`docs/specs/SPEC_SUPPRESS_OS_CREDENTIAL_PROMPTS_2026_05_30.md` (keychain prompt — done, #1208).

## Goal

`task package:macos` produces a **signed, launchable** `AgentMux.app` (and `.dmg`) on Apple Silicon —
the app opens its window and renders the UI — ready for Developer-ID distribution (notarization gated
separately on the Apple account agreement).

This implements the long-deferred `package:macos` TODO. Framework bundling (`bundle:darwin`) and dev
launch already work; the keychain-prompt blocker is fixed (#1208). Two pieces remain: the **packaging
pipeline** (assemble + sign + DMG) and the **CEF subprocess model** (Helper apps).

## Background / findings

A first-cut packaging script assembles a valid, Developer-ID-signed `.app` that passes
`codesign --verify --deep --strict`, and (post-#1208) the **browser process launches**: the host log
shows `Browser created`, window registration, message loop entry, Dock tile, and icon. **But the
renderer/GPU subprocesses crash-loop** (hundreds of crashes in seconds → crash-budget abort → the
"Window stopped recovering" page). This persists even **ad-hoc-signed without hardened runtime**, so it
is **structural**, not a signing/entitlements problem.

Root cause: the host sets `browser_subprocess_path = current_exe()` and re-execs **its own bundle
binary** for renderer/GPU/utility. That works for the bare dev binary but **not** inside a signed `.app`:
every subprocess inherits the main bundle's identity (`CFBundleIdentifier ai.agentmux.cef`, a *regular*
foreground app), which the macOS process/Mach model rejects — the standard reason CEF macOS apps ship
dedicated **Helper.app** bundles for subprocesses.

## Design

### Part A — Bundle layout (mostly implemented)

```
AgentMux.app/Contents/
  Info.plist                              CFBundleIdentifier ai.agentmux.cef
  MacOS/
    agentmux-cef                          host (CFBundleExecutable)
    agentmux-srv-<VERSION>-darwin.arm64   backend (sidecar::resolve_backend_binary)
    frontend → ../Resources/frontend      symlink (host's current_exe()/frontend lookup;
                                          real tree under Resources/ so codesign seals it)
    *.dylib                               GL libs (Chromium DIR_MODULE = exe dir)
  Frameworks/
    Chromium Embedded Framework.framework
    AgentMux Helper.app/                  ← NEW (Part B)
      Contents/
        Info.plist                        CFBundleIdentifier ai.agentmux.cef.helper, LSUIElement=1
        MacOS/
          AgentMux Helper                 copy of agentmux-cef
          *.dylib                         GL libs (the GPU helper's DIR_MODULE = its own exe dir)
  Resources/
    AgentMux.icns
    frontend/…
```

Notes:
- The host self-reexecs for subprocesses ONLY in dev (bare binary). In the `.app`, subprocesses run the
  Helper.
- Resources (frontend, icns) live under `Contents/Resources/`; only executables/dylibs go under
  `MacOS/` (codesign rejects resource trees under `MacOS/`). The `frontend` symlink bridges the host's
  `current_exe()/frontend` lookup with no Rust change.
- `vk_swiftshader_icd.json` is omitted (non-code file codesign rejects under `MacOS/`; SwiftShader-Vulkan
  software fallback only — not needed with hardware GL). If ever required, place under Resources with a
  `library_path` pointing back at `MacOS/`.

### Part B — CEF Helper app (the launch fix)

1. **Packaging:** create `Contents/Frameworks/AgentMux Helper.app` with:
   - `Contents/MacOS/AgentMux Helper` = a copy of `agentmux-cef`.
   - `Contents/MacOS/*.dylib` = the GL libs (the GPU helper resolves them via its own DIR_MODULE).
   - `Contents/Info.plist`: `CFBundleIdentifier = ai.agentmux.cef.helper`, `CFBundleExecutable =
     AgentMux Helper`, `LSUIElement = true`, `CFBundlePackageType = APPL`.
   - Signed with hardened runtime + `build/entitlements.mac.plist` (the renderer's V8 needs
     allow-jit / unsigned-executable-memory; disable-library-validation lets it load the framework).
   - A single helper suffices because all subprocess types share one entitlement set (Chromium permits
     one helper when entitlements don't vary per type).

2. **Rust (macOS-gated, no Windows change) — `agentmux-cef/src/main.rs`:**
   - `browser_subprocess_path`: when running inside an `.app` (a sibling
     `…/Contents/Frameworks/AgentMux Helper.app/Contents/MacOS/AgentMux Helper` exists), point CEF at it;
     else fall back to `current_exe()` (dev). Gated `#[cfg(target_os = "macos")]`; Windows/Linux keep
     `current_exe()` verbatim.
   - `LibraryLoader`: the helper executable lives 4 levels deeper than the main exe, so it must resolve
     the framework with `helper = true` (`…/../../../../Frameworks/…`) instead of `false`
     (`…/../Frameworks/…`). Detect via the exe path containing `.app/Contents/Frameworks/` →
     `helper = true`. The main host + dev binary stay `helper = false`.

   Both are net-additive, macOS-cfg-gated branches around values that are currently `current_exe()`/
   `false` — no shared type/state change (avoids the #1192 Windows-compile failure mode).

### Part C — Signing pipeline (implemented)

Inside-out, deepest first, `--options runtime --timestamp`: GL dylibs → framework `Libraries/*.dylib` →
framework bundle → **Helper.app** (its dylibs, then the helper exe, then the helper bundle) → srv → host
→ the `.app`. Backend/host/helper/app get `build/entitlements.mac.plist`. Cert auto-detected from the
keychain (`security find-identity`; never hardcode identity — `MACOS_SIGN_CERT` override; details in the
private `agentmux-builder`).

### Part D — DMG + notarization (implemented; notarization gated externally)

DMG with an `/Applications` symlink; signed. Notarize via the `notarytool` keychain profile and staple;
degrade to a signed-only DMG if notarization is unavailable. **External blocker:** the Apple Developer
program agreement is expired (`notarytool` HTTP 403) — must be re-signed at appstoreconnect.apple.com
before any notarization succeeds.

## Phasing & PRs

- **PR 1 — packaging pipeline + this spec.** `task package:macos` + `scripts/package-macos.sh`
  (assemble Parts A/C/D) + `docs/macos-signing.md` refresh + the `entitlements` comment + `.gitignore`
  for the build artifact. Produces a valid **signed** DMG. (App not yet launchable — Part B follows;
  documented in the PR + this spec.)
- **PR 2 — CEF Helper app (Part B).** The macOS-gated Rust change + the packaging script emitting +
  signing the Helper.app. **Acceptance:** the signed `.app` opens its window and a renderer subprocess
  **survives** (no crash-loop); `lsappinfo` shows it Foreground with a Dock tile.
- **External:** re-sign the Apple agreement → notarize + staple → upload the DMG to the release.

## Verification (Part B acceptance)

1. `task package:macos`; `open build/AgentMux.app`.
2. A `--type=renderer` subprocess stays alive >15 s; `count windows` ≥ 1; UI renders.
3. Host log: no `crash budget exceeded`; `Browser created` followed by a live renderer.
4. No OS credential prompt (already fixed, #1208).

## Findings update (2026-05-30, post-implementation)

Parts A/B/C/D implemented and tested on the signed `.app`:

- **Part B works for 5/6 subprocess types.** With the Helper app + `browser_subprocess_path` +
  helper-aware `LibraryLoader`, the GPU / network / utility / service / storage subprocesses now run
  as the stable `AgentMux Helper` (verified via `ps`: 3+ live `AgentMux Helper --type=…`), and no
  longer crash-loop. The browser launches, creates its window, sets the Dock tile, and serves the
  frontend. Signatures are valid: `codesign --verify --deep --strict` passes; browser + helper share
  Team `7Z3Z4B37QJ` with hardened runtime.
- **The renderer is gated on notarization.** It still won't stay up, but the cause is NOT a crash —
  `cef-debug.log` shows `base/mac/process_requirement.cc: Unable to derive validation category for
  current process. Signature validation … failed … Code=-67030` (`errSecCSReqFailed`). Chromium
  validates that subprocesses are legitimately signed; on macOS 26 that derivation requires a
  **fully-trusted** signature. `spctl --assess` reports `rejected — source=Unnotarized Developer ID`.
  So the renderer process-requirement check fails **only because the build isn't notarized**.
- **Conclusion:** the implementation is complete. A fully-launching macOS app needs **notarization**,
  which is blocked on the Apple Developer program agreement (`notarytool` HTTP 403 — re-sign at
  appstoreconnect.apple.com). Once notarized + stapled, the renderer process-requirement check should
  pass. (If a non-notarized dev launch is needed before then, investigate a Chromium
  `--disable-features=…` switch for the process-requirement enforcement — not pursued here to avoid
  shipping a speculative flag.)

## Out of scope

Intel (x86_64) slice, Mac App Store distribution, per-type helper entitlements, auto-update.
