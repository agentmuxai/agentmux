# Spec: compress the Linux deb, rpm and portable tarball with xz

**Status:** Active. Item C2 of #4207, approved by the owner 2026-10-14. §§3.1–3.4 ship with this spec; §3.5 is the download page, in its own repo.

## 1. Why

The three Linux formats built with `fpm` or `tar` are still compressed with gzip. The AppImage already uses zstd -22 on the same files. #4207 listed this as C2, about 25–35 MiB per format.

Measured on the nightly `linux-tarball` artifact from 2026-10-13 (version 0.59.22, 456.5 MiB unpacked), recompressing the same tar:

| Compression | Size |
|---|---|
| gzip (today) | 183.0 MiB |
| xz -6 | 135.8 MiB |
| **xz -9e** | **129.7 MiB (−53.3 MiB, −29%)** |

The `.deb` and `.rpm` carry the same files and are 182.0 and 181.9 MiB today, so they should land near the same size. That is an expectation: I couldn't run `fpm` on the machine that measured the tarball, so the first nightly after this change gives the real numbers.

xz over zstd: Debian 11 and Ubuntu 18.04 ship a `dpkg` that can't unpack a zstd `.deb`, and `tar xf` needs `zstd` installed to open a `.tar.zst`. xz-utils is part of essentially every desktop distro, `dpkg` has read xz since 1.15.6 (2010), and rpm has read xz payloads since 4.7.

## 2. What changes for users

- `.deb` and `.rpm` keep their names; only the compression inside changes. They install the same way.
- The portable tarball becomes `AgentMux_<version>_amd64-portable.tar.xz`. `tar xf` opens it where xz-utils is installed (it is, on essentially every desktop Linux; some minimal containers lack it). The folder layout inside is unchanged: a single top-level `AgentMux/`.
- The AppImage, macOS and Windows artifacts don't change.
- Nothing in the app downloads or unpacks these archives.

## 3. Changes, by place

### 3.1 Tarball: `scripts/build-tarball-linux.sh`

- Output name `.tar.xz`. Built with `tar -cf - … | xz -9e -T0 -c`. The script's `set -o pipefail` makes a failing `tar` fail the build.
- Fails early with an install hint when `xz` isn't on `PATH`.
- `-T0` uses every core. `-9e` needs about 674 MiB of memory per thread; the runner has plenty.

### 3.2 deb and rpm: `scripts/build-deb-linux.sh`, `scripts/build-rpm-linux.sh`

- `fpm … --deb-compression xz --deb-compression-level 9` and `fpm … --rpm-compression xz --rpm-compression-level 9`. Both options exist in fpm's `deb.rb` and `rpm.rb`.
- The rpm uses `xz`, not `xzmt`: the multi-threaded form needs a newer `rpmbuild` on the build host, for little gain.

### 3.3 CI: `.github/workflows/build-linux.yml`, `release.yml`

- `build-linux.yml`: the step that locates the packages looks for `…-portable.tar.xz`. Its artifact upload and release upload follow the `tarball_path` output, so nothing else changes there.
- `release.yml`: the publish job's extension allowlist takes `*.tar.xz` in place of `*.tar.gz`. **Without this the tarball would build but be silently left out of the GitHub Release**, as with the Windows `.7z`. Nothing else makes a `.tar.gz` that belongs in a release.

### 3.4 Docs and names

- `Taskfile.yml` task descriptions, `docs/linux.md` and the comment in `crates/cef/src/linux_sandbox.rs` say `.tar.xz`.
- Historical specs and analyses stay as written.

### 3.5 Download page (separate repo: `agentmuxai/agentmux-landing`)

Merge **before** the first release that makes a `.tar.xz`, so the page never lacks the tarball.

- `scripts/fetch-release.mjs`: map `amd64-portable.tar.xz` → `AgentMux_amd64-portable.tar.xz`, add `.tar.xz` to `CONTENT_TYPES`, and treat `.tar.xz` like `.tar.gz` when filling `linux.tarball`. The `.tar.gz` entries stay for older releases.
- `src/components/Download.tsx` and `FeaturesPage.tsx`: the label says the format actually served.

## 4. Order

1. Landing PR: accepts both `.tar.gz` and `.tar.xz`. Safe to merge first.
2. This repo's PR (§§3.1–3.4).
3. After the first release: read the real sizes of the `.deb` and `.rpm` from the release, and update this spec and #4207.

## 5. Verification

- Measured: xz -9e on the real payload (§1). The new `tar | xz` pipeline round-trips a sample tree byte for byte, extracts with `tar xf`, and a failing `tar` exits non-zero.
- On the first nightly: the `linux-deb`, `linux-rpm` and `linux-tarball` artifacts build. Check `dpkg-deb --info`, `rpm -qp --info` (payload compressor `xz`), and that `tar xf` opens the tarball.
- On the first release: all three are in the GitHub Release and on the download page.

## 6. Risks

- **`fpm` flags untested locally.** If a flag is rejected, the Linux job fails loudly on the first nightly; no wrong package ships.
- **Slower builds:** xz -9 is single-threaded in `fpm` and takes a few minutes longer than gzip. Acceptable for a nightly.
- **Minimal systems without xz-utils** can't open the tarball until it's installed. The AppImage and the packages don't need it.
