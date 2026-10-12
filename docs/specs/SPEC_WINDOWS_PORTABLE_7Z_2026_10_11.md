# Spec: ship the Windows portable as `.7z` instead of `.zip`

**Status:** Active. Approved by the owner 2026-10-11. §§3.1–3.3, 3.5 and 3.6 ship with this spec; §3.4 is the download page, in its own repo.
Tracking: #4207 (package size), item C1.

## 1. Why

The Windows portable is the one download still compressed with Deflate.
Built from the same folder, the `.zip` is 194.1 MiB and the Inno Setup
installer (LZMA2, solid) is 140.2 MiB (the #4207 measurement). An LZMA or zstd
`.zip` is not an option: Windows 10 Explorer can't open one, so it would look
like a normal zip and fail. [`docs/specs/cef-size-reduction.md`](cef-size-reduction.md) already
recommended `.7z` for Windows.

Measured on the 0.59.19 portable (`e3a741d`, 370 files, 474.6 MiB unpacked),
with the settings in §3.1:

| Archive | Size |
|---|---|
| `.zip` (today, Deflate) | 205.1 MiB |
| `.7z` (solid LZMA2, `-mx=9`) | **126.5 MiB (−78.6 MiB, −38%)** |

The `.7z` passed `7z t`, and its file list and sizes match the folder exactly.
Compressing took about 4½ minutes with the 32-bit standalone `7zr.exe`; the
64-bit `7z` on the CI runner should be faster.

## 2. What changes for users

- The release asset `agentmux-<label>-x64-portable.zip` becomes
  `agentmux-<label>-x64-portable.7z`, with the same layout inside: the
  launcher and `runtime/` at the archive root, no wrapping folder.
- Windows 11 (23H2 and later) opens `.7z` in Explorer. Windows 10 needs
  7-Zip (or another archiver). The download page and the bundled
  `README.txt` say so. The setup `.exe` stays the download for anyone who
  doesn't want an archive.
- macOS and Linux artifacts don't change.
- Nothing in the app downloads or unpacks the portable (`install_update` is a
  stub, and no Rust code picks a release asset), so installed versions need
  no compatibility path.

## 3. Changes, by place

### 3.1 Producer: `scripts/package-portable.sh` (the only place the archive is made)

- Name: `agentmux-$LABEL-x64-portable.7z` (`ZIPPATH` → `ARCHIVE_PATH`, and
  the `ZIP_*` variables to `ARCHIVE_*`).
- Make it with 7-Zip, run from inside the portable folder so the root stays
  flat: `7z a -t7z -m0=lzma2 -mx=9 -ms=on -mmt=on <archive> .`
- Find 7-Zip on `PATH` (`7z`, then `7za`), then in `$PROGRAMFILES/7-Zip/7z.exe`
  (winget and the 7-Zip installer don't add it to `PATH`).
- **No fallback to `.zip`.** If 7-Zip is missing or fails, stop with an error
  that says how to install it (`winget install 7zip.7zip`). A silent fallback
  would ship a different file under a different name, which is the failure
  mode the current "fail when every method fails" check exists to prevent.
- After writing it, check the archive with `7z t`, so a truncated archive
  fails the build.
- The success output says `Archive:` instead of `ZIP:`.
- The bundled `README.txt` says "Extract this folder (or the .7z archive)
  anywhere", and that Windows 10 needs 7-Zip to open the archive.
- The `VERSION_HISTORY.md` size row: its awk matches the table header
  `| Version | Date | ZIP`. The header is not renamed (feature PRs don't edit
  `VERSION_HISTORY.md`), so the regex stays, and the column now holds the
  archive's size. The row is only written by release builds.
- Comments in [`scripts/package.sh`](../../scripts/package.sh) that say "ZIP" are updated.

### 3.2 CI: `.github/workflows/build-windows.yml`

- A step before packaging checks that 7-Zip is there (`7z i`). The
  `windows-latest` image ships 7-Zip on `PATH`. The check makes a future image
  change fail early, with a clear message, instead of at the end of a
  25-minute build.
- `upload-artifact` `windows-portable`: path `agentmux-*-x64-portable.7z`
  (`if-no-files-found: error` stays, so a missing archive fails the job).
- `gh release upload`: `agentmux-*-x64-portable.7z`.
- `upload-artifact` always zips what it uploads, so the nightly artifact is a
  `.7z` inside a `.zip`. That's acceptable for CI artifacts.

### 3.3 CI: `.github/workflows/release.yml`

- The publish job's extension allowlist (the `find … -name "*.zip" …` line):
  add `*.7z` and remove `*.zip`. **This is the change that matters most.**
  Without `*.7z` the archive builds fine but is silently left out of the
  GitHub Release, and so off the download page. Nothing else produces a
  `.zip`, so removing it keeps a stray one from being published.

### 3.4 Download page (separate repo: `agentmuxai/agentmux-landing`)

Ship this **before or with** the first release that makes a `.7z`, so the
page never points at a missing asset.

- `scripts/fetch-release.mjs`: `UNVERSIONED_NAMES` gets
  `'portable.7z' → 'agentmux-x64-portable.7z'`, next to the zip entry, which
  stays so older releases still mirror. `CONTENT_TYPES` gets
  `.7z → application/x-7z-compressed`. The `name.includes('portable')` rule
  already maps either one to `windows.portable`.
- `src/components/Download.tsx`: the Portable entry shows the format, with a
  note that Windows 10 needs 7-Zip.
- Copy that says "portable ZIP" (`Features.tsx`, `FeaturesPage.tsx`,
  `SourcesPage.tsx`) says "portable archive". Sizes quoted as
  "~190-205MB portable" are updated once the first `.7z` release is measured.
- The old unversioned mirror `releases/latest/agentmux-x64-portable.zip` keeps
  serving the last zip until it's removed. Delete it in the first `.7z`
  release's deploy, so nobody downloads a stale version from it.

### 3.5 Developer tooling

- [`scripts/check-toolchain.sh`](../../scripts/check-toolchain.sh) (Windows): report whether 7-Zip is found, the
  same way `package-portable.sh` looks for it. Missing is a note ("needed for
  `task package`"), not a failure, because `task dev` doesn't need it.
- [`scripts/bootstrap.ps1`](../../scripts/bootstrap.ps1) doesn't install it: winget's 7-Zip doesn't land on
  `PATH`, and only packaging needs it.

### 3.6 Docs

- `README.md`: the output of `task package`, the nightly and release download
  tables, and "a portable ZIP".
- `BUILD.md`: the portable output and size lines, the artifact table, and the
  stale NSIS mention (the installer is Inno Setup).
- [`.changesets/README.md`](../../.changesets/README.md): the example file name.
- [`docs/specs/app-update-check.md`](app-update-check.md): the unimplemented portable update flow
  names a `.zip` asset; it becomes the `.7z`.
- [`docs/linux.md`](../linux.md) and `SPEC_LINUX_DISTRO_TARGETS_AND_DOWNLOADS_PAGE_2026_09_15.md`:
  "Windows portable ZIP" comparisons.
- Historical analyses and retros ([`docs/analysis/portable-zip-ci-vs-local.md`](../analysis/portable-zip-ci-vs-local.md),
  [`docs/retro/2026-04-12-portable-size-audit.md`](../retro/2026-04-12-portable-size-audit.md), …) stay as written.

### 3.7 Unchanged

- The setup `.exe` and the MSIX are built from the portable folder, not the
  archive.
- The CEF runtime download in `build-windows.yml` (`Expand-Archive` of the CEF
  zip) is an input, not our archive.
- No checksum file or signing step exists for the portable in this repo. The
  landing site's `checksums.sha256` covers it through the mirror (§3.4).
- No test asserts the archive's name. The e2e tests use the extracted folder,
  whose name doesn't change.

## 4. Order

1. Landing PR (§3.4): accepts both `.zip` and `.7z`. Safe to merge first.
2. This repo's PR (§§3.1–3.3, 3.5, 3.6): the producer and CI change together.
   A nightly or release build after it makes a `.7z`.
3. After the first release with a `.7z`: measure it, update the size copy on
   the landing page, and delete the stale `latest` zip mirror.

## 5. Verification

- Local: `task package` makes a `.7z`, `7z t` passes, and extracting it gives
  the same file list as the portable folder (`7z l -slt` against `find`). The
  extracted app launches.
- Size: record the `.7z` size next to the setup `.exe`'s. Measured once
  already (§1): 126.5 MiB.
- CI: a `build-windows.yml` run uploads `windows-portable` holding the `.7z`.
- Release: a dry run of the allowlist `find` over a sample artifact tree
  includes the `.7z`.
- Missing 7-Zip: `package-portable.sh` with 7-Zip hidden fails with the
  install hint, not a zip.

## 6. Risks

- **Windows 10 users without 7-Zip** can't open the archive. Mitigations: the
  note on the download page and in `README.txt`, and the setup `.exe` stays
  available. A self-extracting `.7z.exe` was rejected: unsigned, so
  SmartScreen warns, and it adds a third Windows download.
- **The landing page ships late:** the release would publish a `.7z` the page
  doesn't know, and the page would keep the last zip. §4's order prevents it.
