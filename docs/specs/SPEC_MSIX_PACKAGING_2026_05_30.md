# SPEC: MSIX Packaging for the Microsoft Store

**Date:** 2026-05-30 · **Author:** AgentX · **Target version:** 0.40.1 · **Status:** Draft (pre-implementation)

> **Why this spec exists:** the only MSIX tooling in the org is **Tauri-era and non-functional** on
> the current app — `agenty/msix-packaging` (`scripts/package-msix.ps1` + `src-tauri/AppxManifest.xml`,
> v0.32.25) stages `src-tauri/`, `WebView2Loader.dll`, `agentmuxsrv-rs.exe`, `wsh.exe`, and the
> `agentmuxai/agentmux-builder` repo only has a `tauri-build.yml`. None apply since the CEF rewrite
> (no Tauri / WebView2 / wsh). `task package:msix` is a TODO stub. This spec defines MSIX packaging
> for the **current** 100%-Rust + bundled-CEF architecture and is the basis for implementation.

---

## 1. Goal

Produce a **Store-ready `.msix`** from the `task package` portable build that can be uploaded to
the AgentMux listing in Microsoft Partner Center, plus a repeatable `task package:msix` command and
a local-install validation path.

## 2. Verified facts (checked 2026-05-30 against the v0.40.1 portable)

| Fact | Verified value |
|------|----------------|
| Install image | `task package` portable: **root `agentmux.exe`** (the launcher) + **`runtime/`** = host `agentmux-0.40.1.exe`, `agentmux-srv-0.40.1-windows.x64.exe`, `libcef.dll`, `libEGL.dll`, `libGLESv2.dll`, `chrome_elf.dll`, `d3dcompiler_47.dll`, `*.pak`, `locales/`, `icudtl.dat`, `v8_context_snapshot.bin`, `frontend/`, `resources/`, `tools/` |
| Also in portable | `data/`, `agentmux-portable.marker`, `README.txt` — **must NOT be packaged** (see §4) |
| Entry point | root **`agentmux.exe`** = the launcher (Job Object, single-instance named pipe, spawns host + srv from `runtime/`) |
| Tooling | `makeappx.exe` + `signtool.exe` present at `C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\` |
| Icon sources | `assets/linux/icons/hicolor/{16,32,48,64,128,256,512}/apps/agentmux.png`, `agentmux-cef/resources/win/agentmux.ico`, `assets/agentmux-logo-brain-alternate.png` |
| Current version | `0.40.1` → MSIX 4-part `0.40.1.0` |

## 3. Architecture note — why this is a FullTrust packaged desktop app

AgentMux is a native desktop app that spawns child processes (terminals, agent CLIs, the srv
sidecar, the CEF host). It is **not** a UWP/sandboxed app. It must ship as an MSIX **`Windows.FullTrustApplication`**
with the **`runFullTrust`** restricted capability. This is the Desktop-Bridge model — the package is
just a delivery/identity wrapper around the existing portable binaries; the app runs with normal
desktop privileges.

## 4. Package contents (staging rules)

Stage **the entire portable EXCEPT**:
- ❌ `agentmux-portable.marker` — **critical.** Its presence puts the app in *portable mode*, which
  writes its data dir **alongside the executable**. Under MSIX the install dir
  (`C:\Program Files\WindowsApps\<PackageFullName>\`) is **read-only**, so portable mode would fail
  to write and the app would break on first launch. Excluding the marker makes the app run in
  **installed mode** → data dir = per-user `%USERPROFILE%\.agentmux` (writable, outside the package).
- ❌ `data/` — the portable's seed data dir; not shipped (installed mode creates its own).
- ❌ `README.txt` — not needed in the package.

> **✅ VERIFIED (2026-05-30):** `agentmux-common/src/runtime_mode.rs` resolves the runtime mode in
> priority order **portable-marker → dev-exe-path → installed** (`RuntimeMode`, `is_portable_marker_present`).
> The marker check is `<exe-dir>/agentmux-portable.marker` (with a bundle-root fallback). With the
> marker **absent** and the MSIX install path (`C:\Program Files\WindowsApps\…`) not matching the
> dev-exe heuristic, the app resolves to **Installed** → platform-default per-user data dir
> (`data_dir.rs`: `portable_root = Some(exe_dir)` only when `mode == Portable`, else `None`;
> `data_dir = common.data_dir`). Unit test `runtime_mode::portable_marker_detection` plus the
> "no marker ⇒ Installed" test confirm this. **Conclusion: excluding the marker is correct and
> sufficient — no launcher change needed.**

Everything else (root `agentmux.exe` + full `runtime/`) is copied verbatim into the package root.

## 5. AppxManifest.xml (design)

```xml
<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="rescap">

  <Identity
    Name="AgentMux.AgentMux"          <!-- ✅ recovered from published PFN AgentMux.AgentMux_vqr1k32tkfk4y -->
    Publisher="CN=C2BCB530-27CD-4DCF-87C5-2967CE009AAC"  <!-- ✅ verified: hashes to published PFN vqr1k32tkfk4y (from PR #240) -->
    Version="{{VERSION_4PART}}"       <!-- e.g. 0.40.1.0 -->
    ProcessorArchitecture="x64" />

  <Properties>
    <DisplayName>AgentMux</DisplayName>
    <PublisherDisplayName>AgentMux</PublisherDisplayName>  <!-- MUST equal Partner Center publisher display name verbatim, NOT legal entity "AgentMux Corp." (regressed twice — see retro 2026-05-30) -->
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>

  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop"
                        MinVersion="10.0.17763.0"
                        MaxVersionTested="10.0.26100.0" />
  </Dependencies>

  <Resources><Resource Language="en-us" /></Resources>

  <Applications>
    <Application Id="AgentMux" Executable="agentmux.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="AgentMux"
        Description="Integrated agentic workflow environment — run multiple AI agents side by side"
        BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png"
        Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile Wide310x150Logo="Assets\Wide310x150Logo.png"
                         Square71x71Logo="Assets\Square71x71Logo.png"
                         Square310x310Logo="Assets\Square310x310Logo.png"
                         ShortName="AgentMux" />
      </uap:VisualElements>
    </Application>
  </Applications>

  <Capabilities>
    <rescap:Capability Name="runFullTrust" />        <!-- spawns terminals/agents/CLIs + native host -->
    <Capability Name="internetClient" />
    <Capability Name="internetClientServer" />
    <Capability Name="privateNetworkClientServer" /> <!-- srv binds localhost; packaged app is loopback-exempt for its own processes -->
  </Capabilities>
</Package>
```

Changes vs the stale Tauri manifest: `Executable` stays `agentmux.exe` (the portable root launcher
*is* named `agentmux.exe`, so this lines up), **WebView2 dependency removed** (CEF is bundled),
version → `0.40.1.0`, identity tokenized for the build script to patch.

## 6. Visual assets

Generate the Store logo set from the 512×512 source (`assets/linux/icons/hicolor/512x512/apps/agentmux.png`):
`StoreLogo.png` (50×50), `Square44x44Logo.png`, `Square71x71Logo.png`, `Square150x150Logo.png`,
`Square310x310Logo.png`, `Wide310x150Logo.png` (logo centered on transparent/branded field).
Commit the generated PNGs under `packaging/msix/assets/` so the build is reproducible without an
image toolchain on the build host. (Scale-variant `.scale-200` assets optional; single-scale is
accepted by the Store.)

## 7. Build pipeline

**`scripts/package-msix.ps1`** (rewrite of the stale script):
- Params: `-PortableDir` (default: most recent `~/Desktop/agentmux-*-x64-portable/`, or build fresh
  via `task package` if absent), `-Name`, `-Publisher`, `-PublisherDisplayName`, `-OutputDir` (default `dist\msix`), `-Sign` (self-sign for local test).
- Steps:
  1. Resolve `makeappx.exe` (highest SDK under Windows Kits).
  2. Read version live from `package.json` → `X.Y.Z.0`.
  3. Stage portable into `dist\msix\staging\` per §4 exclusions.
  4. Copy `packaging/msix/assets/*` → `staging\Assets\`.
  5. Render `packaging/msix/AppxManifest.xml.template` → `staging\AppxManifest.xml`, substituting Name/Publisher/Version/PublisherDisplayName.
  6. `makeappx pack /d staging /p dist\msix\AgentMux_<ver>_x64.msix /overwrite`.
  7. If `-Sign`: create/trust a self-signed cert whose subject == `Publisher`, `signtool sign /fd SHA256`.
- Wire **`task package:msix`** → invoke this script (replace the TODO echo).

## 8. Microsoft Store submission

1. **Reserve the app name** + obtain **Publisher ID** (`CN=…`) in Partner Center → Account settings →
   Legal info → Publisher ID. **These are required and only the maintainer has them** — they gate
   §5's `Identity Name` / `Publisher`. *(Open item, §10.)*
2. `runFullTrust` is a **restricted capability** → the submission must include a justification ("desktop
   app spawns user-authorized terminal/agent child processes; not sandboxable").
3. **Signing:** the Store **re-signs** on ingestion — the uploaded `.msix` does **not** need our cert.
   Self-signing is only for local install testing.
4. Upload the `.msix` in a new submission; set age rating, description, screenshots.

## 9. Local validation (must pass before Store upload)

Self-sign → trust cert → `Add-AppxPackage dist\msix\AgentMux_0.40.1_x64.msix` → launch from Start menu. Confirm:
- [ ] Launches via `agentmux.exe` (launcher); host (`agentmux-0.40.1.exe`) + srv spawn from `runtime/`.
- [ ] **Data dir is `%USERPROFILE%\.agentmux`** (installed mode), not inside the package. ← validates §4.
- [ ] Window renders (CEF GPU/ANGLE works from the read-only `WindowsApps` dir).
- [ ] Terminals + agent CLIs spawn (child-process creation under FullTrust).
- [ ] srv reachable on localhost (loopback) from the host.
- [ ] Single-instance named pipe + Job Object behave (no double-launch / orphan).
- [ ] Close + relaunch; uninstall via Settings removes cleanly.

**Highest-risk areas** (expect possible iteration): named-pipe/Job-Object semantics inside the MSIX
container; CEF sandbox + GPU from `WindowsApps`; child processes inheriting package identity; any
absolute-path assumptions in the launcher's `runtime/` resolution.

## 10. Store identity — recovered + remaining

App is **already published** — Store ID **`9P9QCXNNCRK3`**, PFN **`AgentMux.AgentMux_vqr1k32tkfk4y`**.
Recovered from the public Store display-catalog API (`displaycatalog.mp.microsoft.com/v7.0/products/9P9QCXNNCRK3`):

| Manifest field | Value | Status |
|----------------|-------|--------|
| `Identity/@Name` | `AgentMux.AgentMux` | ✅ recovered (PFN prefix) |
| `PublisherDisplayName` | `AgentMux` | ⚠️ MUST equal the Partner Center **publisher display name** verbatim — **NOT** the catalog `DeveloperName` ("AgentMux Corp") and NOT the legal entity. Trusting `DeveloperName` here caused the ingest rejection on 2026-05-30 (regression of bb391461/#240). Guarded in `package-msix.ps1`. See retro 2026-05-30. |
| Package Family Name | `AgentMux.AgentMux_vqr1k32tkfk4y` | ✅ recovered |
| `Identity/@Publisher` | `CN=C2BCB530-27CD-4DCF-87C5-2967CE009AAC` | ✅ recovered from **PR #240** (`f74eb732`, "correct MSIX identity for Partner Center") + **hash-verified** → `vqr1k32tkfk4y` |

**Identity fully resolved — no Partner Center login needed.** The real Publisher `CN` was already
committed to public history by PR #240 and is confirmed *live*: its MSIX publisher-hash equals the
currently-published PFN `vqr1k32tkfk4y`. A Publisher `CN` is an **identity** string (embedded in every
shipped MSIX, derivable from the Store) — **not a secret** — so committing it in the manifest template
in the public repo is correct and standard. (The *signing key* is the secret, and we don't sign — the
Store re-signs on ingest, §8.)

> **Verifiable:** once provided, the build confirms the `CN=…` is correct *before* packaging by
> computing the MSIX publisher-hash of the string and asserting it equals **`vqr1k32tkfk4y`** — a
> mismatch means a wrong Publisher and the Store would reject the upload. The packaging script must
> bake in this guard. Algorithm: SHA-256 of the **UTF-16LE** publisher string → first 8 bytes →
> base32 over `0123456789abcdefghjkmnpqrstvwxyz` (13 chars).
>
> **Caution — do NOT reuse the old local MSIX identity.** `~/Downloads/AgentMux_0.32.48_x64.msix` is
> a *placeholder* test build: `Name="AgentMuxAI.AgentMux"`, `Publisher="CN=AgentMux Corp"`
> (→ hash `651g6xd6qjska`). Neither matches the published Store identity (`AgentMux.AgentMux` /
> `vqr1k32tkfk4y`), so that file is not a source of truth for the real `CN`.

Other (non-blocking) confirmations:
- Min OS target (CEF 146 baseline) — confirm `MinVersion 10.0.17763.0` is acceptable.
- Architecture scope: **x64 only** for now (no arm64 build today).

## 11. Implementation plan (one PR)

Branch `agentx/msix-packaging`:
- `packaging/msix/AppxManifest.xml.template`
- `packaging/msix/assets/*.png` (generated logos)
- `scripts/package-msix.ps1` (rewritten)
- `Taskfile.yml`: wire `task package:msix`
- this spec
- changeset (`patch`)

Then: build a **test MSIX with placeholder identity**, run §9 validation, iterate until green. Swap
in the real Partner Center identity (§10) for the actual Store submission.
