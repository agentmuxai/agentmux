# macOS Code Signing & Notarization

How to build, sign, and notarize the AgentMux macOS app for direct distribution.

The app is a **CEF (Chromium Embedded Framework)** desktop app — 100% Rust, no Tauri/Electron.
`task package:macos` (`scripts/package-macos.sh`) does the full build + sign + (attempted)
notarize in one command; this doc explains what it does and how to set up credentials.

Credential details (Apple ID, Team ID, certificate name) are also kept in the private
[agentmux-builder](https://github.com/agentmuxai/agentmux-builder) repo.

---

## Prerequisites

- A **Developer ID Application** certificate in your login Keychain
  (`security find-identity -v -p codesigning` should list a
  `Developer ID Application: <Your Name> (<TEAMID>)` entry). The packager
  auto-detects it; the team's actual identity details live in the private
  `agentmux-builder` repo — never hardcode them here.
- An **app-specific password** stored as a `notarytool` Keychain profile (one-time, below).
- Xcode Command Line Tools (`codesign`, `xcrun notarytool`, `iconutil`, `sips`, `hdiutil`).

---

## One-Time: Store Notarization Credentials

```bash
xcrun notarytool store-credentials "notarytool" \
  --apple-id "<your-apple-id>" \
  --password "<app-specific-password>" \
  --team-id  "<your-team-id>"
```

App-specific passwords: [appleid.apple.com](https://appleid.apple.com) → Sign-In & Security →
App-Specific Passwords (format `xxxx-xxxx-xxxx-xxxx`). After this, reference the profile by
name; the raw password is never needed again.

> **Apple Developer Program agreement.** Notarization requires an in-effect program license
> agreement. If `notarytool` returns `HTTP 403 — "A required agreement is missing or has
> expired"`, sign the updated agreement at [appstoreconnect.apple.com](https://appstoreconnect.apple.com)
> before retrying. Signing still works without it; only notarization is gated.

---

## Build + Sign + Notarize (one command)

```bash
task package:macos          # → ~/Desktop/AgentMux_<VERSION>_arm64.dmg
# task package:macos -- /some/out/dir     # alternate output dir
# NOTARIZE=0 task package:macos           # signed-only, skip notarization
```

`task package:macos` runs `build:host`, `build:backend`, `build:frontend`, `bundle`, then
`scripts/package-macos.sh`, which:

1. **Assembles `AgentMux.app`.** The host resolves everything relative to its own binary
   (`current_exe().parent()`), so the layout needs no Rust changes:

   ```
   AgentMux.app/Contents/
     Info.plist
     MacOS/
       agentmux-cef                          ← host (re-execs itself for renderer/gpu;
                                                no Helper.app needed, --no-sandbox)
       agentmux-srv-<VERSION>-darwin.arm64    ← backend sidecar (resolve_backend_binary)
       frontend/                              ← bundled UI (resolve_frontend_base_url)
       *.dylib + vk_swiftshader_icd.json      ← GL libs (Chromium DIR_MODULE = exe dir)
     Frameworks/
       Chromium Embedded Framework.framework  ← cef-rs ../Frameworks lookup
     Resources/
       AgentMux.icns                          ← generated from the 512px logo PNG
   ```

2. **Signs inside-out** with `--options runtime` (hardened runtime), deepest first — the GL
   dylibs, the framework's `Libraries/*.dylib`, the framework bundle, the `agentmux-srv`
   backend, the host, then the `.app` bundle. The backend + host + app get
   `build/entitlements.mac.plist` (CEF JIT + CLI feature access). Signing each Mach-O before
   sealing the bundle is required for notarization to accept them.

3. **Builds + signs the DMG** (with an `/Applications` drag-target symlink).

4. **Notarizes + staples** via the `notarytool` profile. If notarization is unavailable
   (e.g. the 403 above), it emits a **signed-but-un-notarized** DMG and warns — Gatekeeper
   then needs a right-click → Open on first launch on other Macs until a notarized build ships.

---

## Verify

```bash
DMG=~/Desktop/AgentMux_<VERSION>_arm64.dmg
codesign -dv --verbose=2 "$DMG"                 # Authority should be your Developer ID
spctl --assess --type open --context context:primary-signature -v "$DMG"
# Notarized: "source=Notarized Developer ID".  Signed-only: spctl rejects (expected until notarized).
```

If notarization status is `Invalid`, fetch the log:

```bash
xcrun notarytool log <submission-id> --keychain-profile "notarytool"
```

| Rejection | Fix |
|-----------|-----|
| binary not signed with a valid Developer ID | a nested Mach-O was missed — re-run the packager |
| signature lacks a secure timestamp | ensure `--timestamp` (the packager passes it) |
| hardened runtime not enabled | ensure `--options runtime` (the packager passes it) |

### Verify the artifact is lean (release-correct)

A release DMG must carry **no source maps, no debug symbols, and the `stable` channel**.
`task package:macos` strips all three by default — but verify, especially the channel (a
stray `AGENTMUX_BUILD_CHANNEL_DEFAULT` in the env would bake a dev channel into the build):

```bash
APP=build/AgentMux.app   # the staged .app (inspect before signing seals it, or mount the DMG)
find "$APP" -name '*.map' | wc -l                              # → 0   (source maps stripped)
for b in agentmux-cef agentmux-launcher agentmux-srv-*-darwin.*; do
  echo "$b DWARF: $(otool -l "$APP/Contents/MacOS/"$b | grep -c __DWARF)"   # → 0   (no debug info)
done
strings "$APP/Contents/MacOS/agentmux-cef" | grep -E 'local-[a-z]+-[0-9a-f]{7}'   # → empty (NOT a dev channel)
```

Where the leanness comes from — so you know what to fix if a check fails:

- **Rust binaries** (cef / srv / launcher): stripped at compile time via `Cargo.toml`
  `[profile.release] strip = true` (+ `lto = true`, `opt-level = "s"`). Cargo strips
  `agentmux-srv` even though the packager's `strip -S -x` loop doesn't list it by name.
- **CEF framework + GL dylibs**: `strip -S -x` in `package-macos.sh` (~240 MB of libcef
  symbols → halved), keeping the exported symbols cef-rs's loader needs.
- **Source maps**: `package-macos.sh` deletes `frontend/**/*.map` (~28 MB). `STRIP_MAPS=0`
  keeps them (debugging only).
- **Channel**: with `AGENTMUX_BUILD_CHANNEL_DEFAULT` unset, `option_env!` falls back to
  `stable` → the app opens the real user data dir. `task package:macos` does **not** set it
  (unlike local `task package`, which bakes a `local-<branch>-<hash>` channel), so a clean
  release build is `stable`. Building from the release **tag** (below) keeps it that way.
- **Locales**: non-`en*` Chromium `.lproj` removed (~52 MB).

After stripping + locale trim, the DMG is essentially the irreducible Chromium framework
(~140 MB for arm64); there is nothing further worth stripping.

---

## Upload to the GitHub Release

```bash
gh release upload "v<VERSION>" ~/Desktop/AgentMux_<VERSION>_arm64.dmg --clobber
```

---

## CI / Automated Signing

The [agentmux-builder](https://github.com/agentmuxai/agentmux-builder) repo is intended to host
the automated signing pipeline; it currently lags the CEF migration (see the build-cleanup
tracking issue in the main repo).

## Notes

- **Build from the release tag, not `main`.** `package:macos` uses the committed version (no
  bump), and `main` usually has post-release commits that would otherwise ship under the
  release label. After the `chore: release vX.Y.Z` PR merges:
  `git fetch origin tag vX.Y.Z && git checkout vX.Y.Z`, then build.
- **Don't output to `~/Desktop`.** It's a TCC-protected folder; unless the terminal/process
  has Full Disk Access, the DMG step fails with `rm: …: Operation not permitted`. Pass an
  unprotected output dir: `task package:macos -- "$PWD/release-artifacts"`.
- The notarization keychain profile is named **`notarytool`** (the packager's
  `NOTARY_PROFILE` default) — not `AC_PASSWORD`. Check with
  `xcrun notarytool history --keychain-profile notarytool`.
- You can notarize an already-built signed DMG without rebuilding:
  `xcrun notarytool submit <dmg> --keychain-profile notarytool --wait` → `xcrun stapler staple <dmg>`.
- Notarization typically takes 1–3 minutes. If it stalls >15 min, check
  [Apple's developer status page](https://developer.apple.com/system-status/).
- This is **direct distribution** (outside the App Store). Mac App Store distribution needs a
  different certificate type, provisioning profiles, and App Sandbox entitlements.
