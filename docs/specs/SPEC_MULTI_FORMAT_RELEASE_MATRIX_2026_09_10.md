# Multi-Format, Multi-Arch Release Matrix
**Date:** 2026-09-10
**Status:** Draft, committed for the record — **partially superseded.** Written
2026-09-10 as a full-matrix proposal before any of it was implemented. Phases 2
(`.deb`/`.rpm`) and part of 1 (Linux portable archive, shipped as `.tar.gz`
rather than `.zip`) have since shipped for x86_64 via
`SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md` (#3234) and its
implementation PRs #3236 / #3238 — read that spec first for the Linux-format
work actually in `main`. This document is retained because it is still the only
written analysis of the parts that spec does not cover: Windows `.msi`, macOS
Intel, Linux ARM64, Snap confinement, and the CEF-binary dependency that gates
them. Tracking: #3242.

**Verified 2026-09-16 (#3242):** §2.1's central claim still holds — `agentmuxai/cef`
publishes exactly three binaries (`linux-x86_64`, `macos-arm64`, `windows-x86_64`,
all at `152.0.7977.83`); no `linux-aarch64` or `macos-x86_64` exists at any
version. Two corrections to §2.1 as written: the fork is **public**, not private,
so no special access is needed to check this; and it has **no CI workflows at
all**, so every binary is built by hand — meaning a new arch is a manual
multi-hour Chromium build, not a CI matrix entry. Offsetting that: `patch.cfg` is
shared across all platforms (`CEF_FORK_MAINTENANCE.md`), so an arm64 Linux build
needs **no new patch work** — it reuses the existing `agentmux/7977-*` branches
unchanged.

**Scope:** agentmuxai/agentmux (build/packaging + release CI), agentmuxai/cef (runtime binaries)

---

## 1. Goal

Ship the download matrix a typical multi-platform desktop app (Slack/Discord-style
"All downloads" page) offers, instead of today's single artifact per platform:

| Platform | Format | Arch |
|---|---|---|
| macOS | `.dmg` | ARM64, Intel |
| Linux | `.snap` (Ubuntu/Fedora/Debian/SUSE) | x64, ARM64 |
| Linux | `.deb` (Ubuntu/Debian) | x64, ARM64 |
| Linux | `.rpm` (Fedora/CentOS) | x64, ARM64 |
| Linux | `.AppImage` (portable) | x64, ARM64 |
| Linux | `.zip` (portable, "Porteus") | x64, ARM64 |
| Linux | `.pacman` (Arch Linux) | x64, ARM64 |
| Windows | `.msi` | x64 |
| Windows | `.exe` (installer) | x64 |

This doc inventories what exists today, identifies the real blockers (there is
one big one — CEF runtime binaries — that gates most of the matrix), and
proposes a phased build order so cheap wins ship first and the expensive work
is isolated to its own phase.

---

## 2. Current State (verified against this checkout, HEAD `d6033c8d3`)

| Platform | Format | Arch | Built by | Wired into CI? |
|---|---|---|---|---|
| Windows | `.zip` (portable) | x64 | `task package` → `scripts/package.sh` | Yes (`build-windows.yml`) |
| Windows | `.exe` (Inno Setup installer) | x64 | `scripts/package-installer.ps1` | Yes (`build-windows.yml`) |
| Windows | `.msix` (MS Store) | x64 | `scripts/package-msix.ps1` | Yes, non-blocking (`release.yml`, MS Store job) |
| macOS | `.dmg` | ARM64 only | `scripts/package-macos.sh` (`ARCH="arm64"` hardcoded) | Yes (`build-macos.yml`, `macos-latest` = Apple Silicon runner) |
| Linux | `.AppImage` (portable) | x64 only | `scripts/package-linux.sh` | Yes (`build-linux.yml`, `ubuntu-22.04` runner) |

Nothing else exists: no `.deb`, `.rpm`, `.snap`, `.pacman`, no Linux portable
`.zip`, no Windows `.msi`, no ARM64 Linux, no ARM64/x64 confusion on Windows
(Windows ARM64 isn't in the target matrix above — user's list only asks for
Windows x64 — so it's out of scope here).

### 2.1 The one blocker that gates almost everything: CEF runtime binaries

AgentMux embeds Chromium via a maintained CEF fork
(`agentmuxai/cef`, see `docs/cef-build/CEF_FORK_MAINTENANCE.md`). The runtime
`libcef`/framework binaries are pre-built and published per **(platform,
arch)** pair, and the build scripts download a pinned release tag rather than
building CEF from source on every run (`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md`
pins `cef-windows-x86_64-148.0.7778.180`, `cef-linux-x86_64-148.0.7778.180-codecs`,
`cef-macos-arm64-148.23.25-codecs`).

**Published today: Windows x86_64, Linux x86_64, macOS arm64. Nothing else.**
`agentmux-cef/src/sidecar.rs:535-539` already resolves `arm64` vs `x64`
generically from `target_arch`, and `docs/specs/cef-size-reduction.md` lists
"macOS (arm64 + x86_64)" as a platform set elsewhere — so the *code* doesn't
assume single-arch — but no macOS x86_64, Linux arm64, or Windows arm64 CEF
binary has actually been built and released from the fork. `package-macos.sh`
still hardcodes `ARCH="arm64"` and there is no Linux-arm64 equivalent at all.

Building a new (platform, arch) CEF binary means a full Chromium build (hours,
dedicated CI runner, plus carrying the fork's 4 patches across 18 source files
— see `CEF_FORK_MAINTENANCE.md` §1) — **not** something that falls out of the
agentmux repo's own packaging scripts. This is the real cost center in this
matrix, and it's a per-arch cost paid once (not per packaging format): once
`cef-linux-aarch64` exists, every Linux **format** (deb/rpm/snap/pacman/zip/
AppImage) for arm64 becomes "just" packaging work reusing the same binary.

**Consequence for phasing:** every row below that needs a CEF arch we don't
have yet (macOS Intel, any Linux ARM64 format) is gated behind a CEF fork
release for that arch, tracked as its own upstream workstream — not by this
spec. This spec's own phases order themselves to defer that dependency as
long as possible.

---

## 3. Gap Analysis by Target

### 3.1 Windows `.msi` — packaging-only, no new CEF binary needed

The existing `.exe` is an Inno Setup installer, which is a different format
family from MSI — Inno cannot emit a real `.msi`. Needs a genuinely new
pipeline:

- **Tooling:** WiX Toolset (v5 — MSBuild-based, current) authoring a `.wxs`
  manifest. This is the standard way to hand-roll an MSI for a non-.NET,
  non-MSIX app; there's no shortcut through the existing Inno script.
- **Inputs:** same portable build tree `package-installer.ps1` already
  consumes — no new binary outputs required, just a second installer wrapper
  around the existing portable artifact.
- **Signing:** reuse whatever cert/signing path `.exe`/`.msix` already use
  (check `build-windows.yml` signing steps) — MSI needs the same Authenticode
  signing, not a separate cert.
- **Risk:** low. This is the cheapest item in the whole matrix — no new arch,
  no new CEF binary, well-trodden tooling.

### 3.2 Linux portable `.zip` ("Porteus") — packaging-only

Porteus is a portable-by-design Linux distro; software for it is typically
just a self-contained directory a user unzips and runs — conceptually the
same tree `package-linux.sh` already assembles before squashing it into the
AppImage. This is the second-cheapest item:

- Reuse the AppDir contents `package-linux.sh` builds pre-`appimagetool`,
  `zip` it directly instead of (or in addition to) squashing.
- No new CEF binary for x64. ARM64 version is naturally blocked on the Linux
  ARM64 CEF binary landing (§2.1), same as every other Linux ARM64 row.

### 3.3 Linux `.deb` / `.rpm` / `.pacman` — packaging-only, one shared strategy

None of `cargo-deb`/`cargo-rpm`-style pure-Rust tooling fit here: this isn't a
single Rust binary, it's a mixed Rust + bundled CEF runtime + bundled frontend
tree (same shape the AppImage already wraps). The pragmatic approach is a
generic "take an assembled directory tree and wrap it per target format" tool
rather than three bespoke pipelines:

- **`.deb` + `.rpm`:** [`fpm`](https://github.com/jordansissel/fpm) ("effing
  package management") builds both from the same input directory tree with a
  single invocation per format — this is the standard low-effort way to ship
  a non-native-packaged app as deb+rpm without maintaining two build systems.
  Desktop integration (`.desktop` file, icon) already exists for the AppImage
  path (`docs/specs/linux-icon-and-desktop-2026-05-03.md`) and should be
  reused verbatim.
- **`.pacman`:** `fpm`'s pacman output support is less battle-tested; the
  idiomatic Arch path is a hand-written `PKGBUILD` + `makepkg`. Slightly more
  work than deb/rpm but still packaging-only, no new binary. (Note: Arch users
  conventionally expect an AUR entry rather than a downloadable `.pacman`
  file — worth confirming with whoever owns the downloads page whether AUR
  publication should eventually replace or supplement the raw file download.)
- All three: x64 ships as soon as this is built; ARM64 is blocked on §2.1.

### 3.4 Linux `.snap` — packaging + a confinement decision

Canonical's own compatibility claim (Ubuntu/Fedora/Debian/SUSE via snapd) is
accurate — this is the most cross-distro Linux format in the target list,
which is presumably why it's listed first.

- **Tooling:** `snapcraft`, authoring `snap/snapcraft.yaml`.
- **Confinement:** AgentMux spawns arbitrary agent CLIs/shells as child
  processes with broad filesystem/network access — this is squarely
  `classic` confinement territory, not `strict`. Classic confinement on the
  Snap Store requires manual review/allowlisting by Canonical, which has
  unpredictable lead time and is a one-time approval gate outside this repo's
  control.
- **De-risking option:** a `.snap` file can be distributed for direct
  download and installed with `snap install --dangerous <file>` without ever
  going through Snap Store review — this satisfies "downloadable `.snap` on
  our own downloads page" without blocking on Canonical's review queue. Store
  presence (so `snap install agentmux` works) would be a separate, later
  step.
- x64 first; ARM64 blocked on §2.1 same as the rest of Linux.

### 3.5 macOS Intel `.dmg` — blocked on a new CEF binary

- **Blocker:** no `cef-macos-x86_64` fork release exists (§2.1). Rosetta 2
  translates x64→arm64, not the reverse, so an Intel Mac cannot run an
  arm64-only build — this has to be a real native x86_64 CEF binary, not a
  compatibility shim.
- **Once the CEF binary exists:** `package-macos.sh`'s hardcoded `ARCH="arm64"`
  assignment needs to become a parameter, `build-macos.yml` needs a second job on an
  Intel runner (GitHub-hosted `macos-13` is the last Intel image; Apple
  Silicon has been the default `macos-latest` for a while, matching the
  comment already in `build-macos.yml:63`), and `Taskfile.yml`'s
  `dist/bin/agentmux-srv-{{.VERSION}}-darwin.arm64` backend-build step needs
  an x86_64 counterpart.
- **Universal binary option (not recommended as v1):** `lipo`-merging arm64 +
  x64 into a single fat `.dmg` is possible but doubles artifact size and
  build complexity for no benefit here — the target matrix explicitly lists
  ARM64 and Intel as separate downloads, so two DMGs (matching the existing
  per-arch-artifact pattern the rest of this matrix already uses) is simpler
  and consistent.

### 3.6 Linux ARM64 (all formats) — blocked on a new CEF binary

Same shape of blocker as §3.5: no `cef-linux-aarch64` fork release exists.
Once it does:

- Rust cross-compilation target `aarch64-unknown-linux-gnu`. Two paths:
  **(a)** cross-compile from an x64 runner with an `aarch64-linux-gnu-gcc`
  toolchain (`cef-dll-sys` builds CEF's C wrapper natively — cross-compiling
  a C++ wrapper is the fiddlier path), or **(b)** build natively on an
  arm64 runner. GitHub now offers `ubuntu-24.04-arm` hosted runners, which
  avoids the cross-compilation toolchain entirely — **(b) is the
  recommended path**, matching how `build-macos.yml` already just runs
  natively on Apple Silicon rather than cross-compiling from Intel.
  Apt dependency install (§ Linux build prereqs, `BUILD.md`) is unaffected —
  same package names, arm64 architecture.
- Once the arm64 backend + host + CEF binary exist, every Linux **format**
  from §3.2–3.4 becomes a matter of pointing the same fpm/snapcraft/makepkg/
  zip packaging at the arm64 build output instead of x64 — no new packaging
  logic, just a second axis on the existing jobs.

---

## 4. Proposed Phasing

Ordered to front-load packaging-only work (no new CEF binary, fast to ship
and validate) and push the two CEF-binary-blocked items (macOS Intel, Linux
ARM64) to the end, where they gate the largest remaining chunk of the matrix
at once rather than blocking early wins.

| Phase | Deliverable | New CEF binary needed? |
|---|---|---|
| 0 | Windows `.msi` (WiX) | No |
| 1 | Linux portable `.zip` (x64) | No |
| 2 | Linux `.deb` + `.rpm` (x64, via `fpm`) | No |
| 3 | Linux `.pacman` (x64, via `makepkg`/`PKGBUILD`) | No |
| 4 | Linux `.snap` (x64, direct-download `--dangerous` path first, Store review as a follow-up) | No |
| 5 | `cef-linux-aarch64` fork release (own workstream, hours-long Chromium build) | — |
| 6 | Linux ARM64 across all of §3.2–3.4's formats (reuses phase 1–4 packaging logic against the phase-5 binary) | Consumes phase 5 |
| 7 | `cef-macos-x86_64` fork release (own workstream) | — |
| 8 | macOS Intel `.dmg`, `package-macos.sh` parameterized, `build-macos.yml` gets an Intel job | Consumes phase 7 |

Phases 0–4 are independent of each other and can ship in any order or in
parallel; the sequence above is by increasing packaging complexity, not a
hard dependency chain. Phases 5–6 and 7–8 are each a hard dependency pair.

---

## 5. Open Questions

1. **Release CI shape:** today `release.yml` fans out to one job per
   (platform) and flattens artifacts by extension glob (`*.zip`, `*.exe`,
   `*.msix`, `*.AppImage`, `*.dmg`). Nine-plus artifacts per release needs
   either more glob patterns in the flatten step or a rethink of that step
   into a manifest-driven upload — worth deciding before phase 0 lands so
   later phases don't each patch the same script.
2. **Snap Store vs. direct download:** does the downloads page need
   `snap install agentmux` (requires Store review, classic-confinement
   approval) or is a downloadable `.snap` file sufficient for launch? Changes
   phase 4's critical path materially.
3. **Arch/AUR expectations:** should `.pacman` eventually be replaced or
   supplemented by an AUR package, given that's the conventional distribution
   channel for Arch users?
4. **Signing coverage:** `.msi` needs the same Authenticode signing as
   `.exe`/`.msix` — confirm the existing cert covers an additional artifact
   without a separate provisioning step. `.deb`/`.rpm`/`.pacman` have their
   own optional GPG-signing conventions (apt/dnf/pacman repos can verify
   package signatures) — decide whether v1 ships unsigned packages (common
   for direct-download-only, pre-repo distribution) or signs from day one.
5. **Self-hosted vs. GitHub-hosted ARM64 runners:** `ubuntu-24.04-arm` hosted
   runners exist but confirm they're available on this repo's plan/billing
   tier before phase 5–6 planning assumes them.

---

## 6. References

- `BUILD.md` — current build prerequisites and commands
- `docs/cef-build/CEF_FORK_MAINTENANCE.md` — CEF fork branch model, why
  cross-platform binary parity is easy to silently lose
- `docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` — current
  pinned CEF binary tags per platform
- `docs/specs/SPEC_MACOS_PACKAGING_2026_05_30.md` — existing macOS DMG
  packaging + signing
- `docs/specs/SPEC_MSIX_PACKAGING_2026_05_30.md` — existing MSIX pipeline
  (closest prior art for a from-scratch Windows packaging format)
- `docs/specs/SPEC_LINUX_APPIMAGE_PER_BUILD_CHANNEL_2026_06_25.md` — existing
  AppImage per-build channel isolation, applies unchanged to any new Linux
  format built from the same tree
- `docs/specs/SPEC_UNIFIED_RELEASE_CICD_2026_06_29.md` — current release
  fan-out this matrix expansion plugs into
- `docs/specs/linux-icon-and-desktop-2026-05-03.md` — existing `.desktop`
  file / icon integration to reuse for deb/rpm/pacman
- `scripts/package-linux.sh`, `scripts/package-macos.sh`,
  `scripts/package-installer.ps1`, `scripts/package-msix.ps1` — existing
  packaging scripts this spec's new scripts should match in structure
  (ephemeral local-build labeling, per-build channel isolation)
