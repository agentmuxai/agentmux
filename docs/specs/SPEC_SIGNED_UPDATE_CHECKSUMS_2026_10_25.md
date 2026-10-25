# Signed checksums for update downloads

**Status:** active. Approved by the operator on 2026-10-25 (decisions of §11 included). Task #99.
Build step 1 (§8) is in progress: the manifest script, the release workflow's signing
job (off until the key exists) and the app's verification module, which nothing calls
yet.
Builds on `SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §4.3 ("a `SHA256SUMS`
signed with a key the app pins is phase 5") and on the update check of #5468
(the latest tag comes from the `releases/latest` redirect, not the API).

## 1. Short answer

Each release publishes a **manifest** (every release file with its size and SHA-256)
and a small signed **envelope** that pins the manifest's hash, all as release assets. The
release workflow makes them from the exact files it is about to upload, with a key
that never lives in the repository. AgentMux ships the matching **public key** and
will not offer, download, unpack or hand to the launcher any release file that the
signed manifest doesn't list with that size and hash. A release without a valid
signature is **not offered**: the panel says why and links the release page.

This replaces "the checksum GitHub lists for the file" (the `digest` in the releases
API) as the thing the download is checked against. That digest is computed by GitHub
for whatever is uploaded, so it proves the file arrived whole, not who built it.

It also removes the update check's last reason to call the API: names, sizes and
checksums come from the manifest, fetched from `github.com/.../releases/download/`
like the release files themselves.

## 2. Threat model

What this defends against: a release (or one file in it) that did not come from our
build and signing step, however it got there: a stolen release-publishing token, a
compromised account with write access to the repository, a maintainer's mistaken or
malicious upload, a swapped asset after the release was made, a mirror or proxy that
rewrites the API's answer.

What it does not defend against, and says so:
- the signing key itself being stolen (§6 covers replacing it, but not the releases
  signed before it is noticed);
- a compromised build that is then signed (the manifest says who signed, not that
  the code is good);
- hiding a new release (a stale `latest` redirect): the app keeps the old version,
  which is safe, not current;
- a local attacker who can already run code as the user.

## 3. The manifest

Three release assets next to the files, named by version so a download never mistakes
one release's for another's:

- `AgentMux-<version>-manifest.json`: the list of files. It can grow with the
  release; nothing limits its size but a sanity cap in the app (1 MB).
- `AgentMux-<version>-envelope.json`: a small fixed document that names the
  manifest by its hash. **This** is what is signed.
- `AgentMux-<version>-envelope.json.sig`: the signature.

```json
{
  "format": 1,
  "version": "0.60.1",
  "tag": "v0.60.1",
  "files": [
    { "name": "AgentMux-0.60.1-x64-setup.exe", "size": 151460383, "sha256": "…" },
    { "name": "agentmux-0.60.1-x64-portable.7z", "size": 129471877, "sha256": "…" }
  ]
}
```

```json
{
  "format": 1,
  "tag": "v0.60.1",
  "manifest_sha256": "…",
  "created_at": "2026-10-25T12:00:00Z"
}
```

The signature file is one line per signature, `<key id> <base64 Ed25519 signature>`,
over the **exact bytes** of the envelope file. There is no canonical form to get
wrong: the app verifies the bytes it downloaded and only then parses them. The
manifest is then accepted only if the SHA-256 of its exact bytes equals
`manifest_sha256`.

Signing an envelope rather than the manifest keeps the signed message fixed in size
(about 200 bytes, small enough for any signing service) however many files a
release adds, and keeps what the key signs trivially auditable.

The envelope names its `format` version and the release `tag` inside the signed body,
and the manifest repeats the tag and version, so neither can be changed or replayed
onto another release without breaking the signature or the hash.

Why this and not minisign, cosign or GPG: Ed25519 is already in the tree (jekt
signing, `crates/common/src/jekt_sign.rs`), so the app gains no dependency; the
format is a page long; and the CI side is one `openssl pkeyutl -sign -rawin` (or
Node's `crypto.sign`) call. Why not Sigstore keyless: it would make the app trust a
transparency service and a certificate chain it has to carry and keep working
offline; a pinned key is simpler to reason about and has no network dependency at
verify time.

## 4. Release workflow

A separate `sign-release` job runs after the builds and before `Publish release`; it
gathers the same files (`dist-flat/`) that `publish` will, and does:

1. Build the manifest from `dist-flat/*` (name, size, SHA-256), with the version and
   tag the job already has, and the envelope that names it by hash.
2. Sign the envelope with the release signing key.
3. Hand manifest, envelope and signature to `Publish release` as an artifact. That job
   checks again that the files it is about to publish are exactly the ones the
   manifest lists, then uploads all of them in the same `gh release` command, so no
   moment exists where the files are public and the signature isn't.
4. Verify what was made: the same verification code path the app uses, run against
   the files in `dist-flat/`, with the public key from the repository. The job fails
   on any mismatch, so a signing mistake can't ship.

The key is held by a managed signing service and **never leaves it**: the signing job
may ask it to sign a manifest, once per release, and can't read the key, so a stolen
token or a compromised runner can't take it away (it can only ask for signatures while
the job's own identity is valid, which the service logs). The job authenticates to the
service with the workflow's own short-lived identity token, restricted to release
tags; the repository holds no stored secret for it. The service supports Ed25519
directly, so the app verifies plain Ed25519 (RFC 8032), not a variant. Everything
about the key's creation, policy, the role and its detection rules is written in the
private infrastructure docs (this repository is public, so it names no resource,
account or role). The repository carries the **public** key only. Re-running a publish for an existing tag
re-signs the files it uploads (`--clobber`); the manifest is re-made from what is
actually there, so the pair always matches.

## 5. In the app

### 5.1 Pinned keys

`UPDATE_KEYS` in `crates/common` (read by srv and by the launcher): a short list of
`(key id, Ed25519 public key)`. At most two at a time (§6).

### 5.2 Check

After the tag is known (`update_check.rs`), fetch the manifest and its signature from
`https://github.com/agentmuxai/agentmux/releases/download/<tag>/` (the same prefix the
download is limited to, `RELEASE_DOWNLOAD_PREFIX`; redirects only to GitHub's own
download hosts; size-capped reads): the envelope and its signature first, the manifest
only once the envelope has passed. Then:

1. Take the signature lines whose key id is pinned. None: unsigned (§7).
2. Verify the envelope bytes against one of them. Failure: bad signature (§7).
3. Parse the envelope. Require `format` known and `tag` equal to the tag from the
   redirect (so an old signed envelope can't be replayed under a newer tag).
4. Fetch the manifest and require the SHA-256 of its exact bytes to equal
   `manifest_sha256`. Failure: bad signature (§7), since the manifest isn't what was
   signed.
5. Parse the manifest. Require `format` known, `version`/`tag` equal to the envelope's,
   every `name` a plain file name, every `sha256` 64 hex digits.
6. The status's `asset` (the file for this install kind) now comes from the manifest:
   name, size and SHA-256 are the signed ones. The download URL is the release's, built
   from the tag and the name, not taken from anywhere else.

The releases API is still used for the notes and the published date (and the upgrade
floor in the notes), but is no longer needed for anything the download trusts. If it
is limited or down, the check still offers the signed download.

### 5.3 Download and unpack

`update_download.rs` already hashes as it receives and requires an exact size and hash;
those two values now come from §5.2. Nothing is unpacked, moved or run before the hash
matches. The 7z/tarball is unpacked only after that, as today.

### 5.4 Launcher

For an installer (`setup.exe`, and later `dmg`, `.deb`/`.rpm`) the launcher already
re-hashes the staged file against the `sha256` srv passes it
(`UpgradeTarget::Installer`, `upgrade_restart.rs`). That value is now the signed one.
The launcher does not repeat the signature check: srv checked it, and a process that
can alter the arguments srv gives its own launcher can already do worse (§2).

### 5.5 Per file kind

| Install kind | File | Verified against the signed manifest at |
|---|---|---|
| Windows `setup.exe` | `AgentMux-<v>-x64-setup.exe` | download (size + hash), launcher re-hash before running |
| Windows portable | `agentmux-<v>-x64-portable.7z` | download, before unpacking |
| Linux AppImage | `AgentMux_<v>_amd64.AppImage` | download, before it is made executable and run |
| Linux portable | `AgentMux_<v>_amd64-portable.tar.xz` | download, before unpacking |
| macOS, `.deb`, `.rpm`, `.msix` | listed in the manifest | covered by the same manifest when 3c/3g/Store use it |

Code signatures (Authenticode, notarization) are separate and unchanged; they say who
published the binary to the OS, the manifest says it is the file our pipeline made.

## 6. Keys and rotation

- **Two slots.** The app pins up to two keys: the current (`k1`) and, once prepared, the
  next (`k2`). A manifest's signature file can carry a line for each.
- **Rotate** (planned): (1) ship an app version that pins `k1` and `k2`; (2) when
  enough of the installed base has it (§7 table), publish releases signed by both,
  then by `k2` alone; (3) a later app drops `k1`. An app older than step 1 can't
  verify a release signed only by `k2` and is told "not offered", so step 2's dual
  signing runs for as long as the oldest supported version needs (the upgrade floor,
  `UPGRADE_FLOOR`, already bounds that).
- **Compromise**: publish a release signed by the next key with an app that no longer
  pins the old one. Apps that haven't taken it still trust the old key, which is the
  honest limit of a pinned key; the mitigation is the dual-signing window above kept
  short, and the release page's note.
- Key generation, storage, who may sign and the rotation drill are written down in the
  private infrastructure docs. Nothing about the key's location, names or the cloud
  it lives in belongs in this repository or its PRs.

## 7. What the person sees

| Situation | State | Message |
|---|---|---|
| Signed, valid, newer | `available`, download offered | as today |
| Release has no manifest or no pinned key's signature (every release before the first signed one) | `available`, no download | "v0.59.40 wasn't signed, so AgentMux won't download it itself. Get it from the release page." (link) |
| Signature doesn't verify | `available`, no download | "The signature on v0.60.1 doesn't match, so AgentMux won't download it. Don't install it; tell the AgentMux team." (and logged as an error) |
| Manifest doesn't list the file for this install | `available`, no download | "v0.60.1 has no file for this kind of install." |
| GitHub unreachable | as today | as today |

A release page link is always shown; nothing is ever downloaded unverified.

## 8. Rollout (so it can't strand anyone)

An app that doesn't know about signatures keeps using the API digest, and keeps
working. The first signed release has to be offered to apps that can't check it, and
apps that can check it must not be locked out by a release that isn't signed.

1. **Sign only.** The workflow publishes manifest and signature for every release;
   nothing reads them yet. Proves the key, the workflow and the verify step (§4.4).
2. **Verify, don't enforce.** An app version that pins the key checks the manifest and
   logs the result, and still uses the API digest. Run for one release.
3. **Enforce.** The next app version uses only the signed values and shows §7's
   messages. From here a release that fails to sign is not offered, which is the point;
   the workflow's own verify step (§4.4) makes that a failed publish, not a stranded
   user.

Anyone on a version before step 2 upgrades as today, then is covered.

## 9. Tests

- Pure: envelope and manifest parsing (format, name safety, hash shape, version/tag
  mismatch); signature verification over the envelope (valid, wrong key, no pinned key,
  tampered byte, extra lines, second key slot); a manifest whose hash isn't the
  envelope's; a manifest that grew past the old 4 KB; the status for each §7 row.
- Download: a file of the wrong size or hash against a signed manifest is deleted and
  refused (reusing the download tests); a name not in the manifest is refused.
- Workflow: a script test that builds manifest and envelope from a fixture folder and
  verifies them with the same code the app uses, plus one that must fail on a swapped
  file.
- Live (after step 1): the published manifest of a real release verifies with the
  shipped public key.

## 10. Not in this spec

- Authenticode, notarization, MSIX (unchanged).
- Signing the `.deb`/`.rpm` repositories (Linux packaging, 3g).
- A revocation channel for a compromised key beyond §6.
- Replacing the notes/date lookup (API) with a manifest field; it needs no trust.

## 11. Decisions

Taken by the platform lead on 2026-10-25, pending the operator's OK:

1. **Key custody:** a managed signing service the signing job can call but never read,
   reached with the workflow's identity token (no stored secret). Creating it is a
   production change that needs the operator's OK; the design is in the private
   infrastructure docs. If the service didn't offer Ed25519, the fallback would be
   ECDSA P-256 and §3/§5 would change to match; it does offer it.
2. **Enforce** from step 3: a download that can't be verified is not offered.
3. **Two key slots from day one.**
4. **Manifest format:** the JSON above, with a `format` version and the release `tag`
   inside the signed body. The platform lead's change: the signed thing is a small
   fixed envelope (`format`, `tag`, `manifest_sha256`, `created_at`), and the manifest
   is checked by its hash, so the manifest can grow as releases add files.
