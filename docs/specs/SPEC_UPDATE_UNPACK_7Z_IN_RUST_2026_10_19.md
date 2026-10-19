# Spec: the Windows upgrade unpacks the release's .7z itself, not with tar.exe

**Status:** active. Built in #5124 (srv decodes the .7z with sevenz-rust2; the real 0.59.33 portable unpacks byte-identical to 7-Zip). Left: the first release with it says, in its notes, that older installs upgrade once by hand (§5).
**Date:** 2026-10-19
**Author:** Lazo
**Verified against:** `agentmux` `main` @ `8313c3a0c` (2026-10-19), and an upgrade from v0.59.32 to v0.59.33 on Windows 10 (19045).
**Builds on:** [SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md](SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md) §4 (phase 3a: downloading and staging a release), [SPEC_WINDOWS_PORTABLE_7Z_2026_10_11.md](SPEC_WINDOWS_PORTABLE_7Z_2026_10_11.md) (why the Windows portable is a `.7z`).

## 1. Ask

The owner, 2026-10-19, trying the in-app upgrade to v0.59.33: "it says update failed with a cut off 'tar...'". Then, given the options: "write a plan to file, then proceed with it". Decoding the `.7z` in srv with the pure-Rust `sevenz-rust2` crate was the recommended option.

## 2. What happened

srv's log, during the upgrade:

```
update: downloading 0.59.33 failed error=tar couldn't unpack it (tar.exe: LZMA codec is unsupported); this Windows may be too old to read .7z files
```

- `update_download.rs` unpacks a Windows portable release with `%SystemRoot%\System32\tar.exe` (libarchive). Its module comment says libarchive "reads the release's LZMA2 `.7z`".
- **This Windows' `tar.exe` is current but built without LZMA.** `tar --version`: `bsdtar 3.8.4 - libarchive 3.8.4 zlib/1.2.5.f-ipp cng/2.0 libb2/bundled`. There's no liblzma in it. It's not an old Windows; it's how Windows 10 ships `tar.exe`. Windows 11 builds may differ, but the updater can't depend on that.
- **The release `.7z` is LZMA2 with the BCJ2 filter.** `scripts/package-portable.sh` packs with `7z a -t7z -m0=lzma2 -mx=9 -ms=on`, and 7-Zip adds BCJ2 for executables (`7z l -slt`: `Method = LZMA2:28 LZMA:20 BCJ2`).
- **Why the test passed:** `update_download.rs`'s unpack test makes its stand-in archive with the same `tar.exe` as a zip ("a zip stands in for the release `.7z`, both go through libarchive"). Zip uses deflate, which that `tar.exe` has; the release's codec was never exercised.

So the in-app upgrade of a Windows portable fails on, at least, every Windows 10.

## 3. Change

**srv decodes the `.7z` itself.** It uses `sevenz-rust2` (Apache-2.0, maintained) with `default-features = false`: decompression only. That adds two crates, `sevenz-rust2` and `lzma-rust2`; `crc32fast` is already in the tree. Its README lists LZMA2 and BCJ2. The Linux `.tar.xz` keeps the system `tar` (with `xz`), unchanged.

- **New module `backend/update_unpack.rs`**, with `unpack_7z(archive, dest)`:
  - walks the archive's entries and writes each file and folder under `dest`;
  - **refuses any entry whose path is absolute, or contains `..`, a drive prefix or a root**, as `tar.exe` did. The archive is already SHA-256-verified against GitHub's release digest; this is defence in depth;
  - runs in `spawn_blocking`, under the existing 10-minute `UNPACK_TIMEOUT`;
  - errors name the entry and the reason ("can't decode the archive: …", "refused an entry outside the folder: …").
- **`unpack_portable`:** on Windows, calls `unpack_7z`; elsewhere, `tar` as now. The rest (the `.partial` folder, `is_staged_portable`, recording the channel, the race with another instance) is unchanged. In particular (QTest, who wrote the updater, 2026-10-19):
  - the download's SHA-256 is still checked against GitHub's digest before anything is unpacked;
  - unpacking still goes into the `.partial` staging folder, and only a complete, checked unpack is moved into place, so a half-unpacked version is never picked up.
- **The module comment** says why: `tar.exe` lacks LZMA on Windows 10.

## 4. Tests

- **A real LZMA2 + BCJ2 fixture:** `crates/srv/src/backend/testdata/update-portable-0.59.27.7z`, a few KB. It's packed with the release's own `7z` flags from a portable-shaped folder (`agentmux.exe`, `runtime/agentmux-portable.marker`, `runtime/agentmux-0.59.27.exe`), so 7-Zip applies BCJ2 to the `.exe` files as it does for the release. The fixture's method list is checked when it's made, and noted in the test.
- `unpack_7z` unpacks it, and `is_staged_portable` accepts the result.
- A crafted entry with `..`, and one with an absolute path, are refused, and nothing is written outside `dest`.
- A truncated archive fails with a decode error, not a panic.
- The existing Windows unpack test switches from the zip stand-in to the fixture, through `unpack_portable`.
- **The real release, by hand:** an ignored test (`AGENTMUX_TEST_PORTABLE_7Z=<path>`) unpacks a downloaded `agentmux-0.59.33-x64-portable.7z` and checks `is_staged_portable(…, "0.59.33")`. It's run once here before the PR, and the result goes in the PR.

## 5. What it doesn't fix

- **Installs on v0.59.33 or older still can't upgrade themselves**, because the updater that runs is the old one. They need one manual upgrade, by extracting the `.7z` with 7-Zip or running `AgentMux-<version>-x64-setup.exe`. The release notes of the first version with this fix say so.
- **The size of the change:** decompression-only `sevenz-rust2` is expected to add roughly 100–300 KB to srv (not measured; the release download is ~127 MB).

## 6. Decisions

- **D1. Decode in srv rather than ship `7zr.exe` or fall back to an installed 7-Zip:** no new binary to ship and license, and it works on any Windows. *Accepted (owner, 2026-10-19).*
