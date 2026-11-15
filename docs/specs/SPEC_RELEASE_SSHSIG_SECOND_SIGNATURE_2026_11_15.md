# Spec: a second signature of each release's envelope, in SSH signature form, so a stock Mac can check it

**Status:** active. Decided 2026-11-15 by the release owner (task #47716). Built in the pull request that carries this file.
**Date:** 2026-11-15
**Author:** Winny (winget)
**Builds on:** [SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md](SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md) (the manifest, the envelope and the Ed25519 signature), [SPEC_ONE_COMMAND_INSTALL_ALL_PLATFORMS_2026_11_01.md](SPEC_ONE_COMMAND_INSTALL_ALL_PLATFORMS_2026_11_01.md) (`install.sh` and `install.ps1`).

## 1. The problem

`install.sh` checks a release the way the app does: the Ed25519 signature of the envelope (which names the manifest's hash), then the manifest's hash and size for the file. The signature check needs `openssl pkeyutl -verify -rawin`, which is OpenSSL 3. A stock Mac's `/usr/bin/openssl` is LibreSSL 3.3.6. Measured on 2026-11-15 on macOS 26.5.2 (Apple Silicon, with only the PATH made stock):

- `openssl genpkey -algorithm ed25519` answers "Algorithm ed25519 not found": the library has no Ed25519 at all (LibreSSL's change log puts it in 3.7.0).
- `openssl pkeyutl` has no `-rawin`.
- `/usr/bin/ruby`'s OpenSSL is the same library, with no Ed25519 key class; `python3` and `swift` are stubs that open the "install developer tools" dialog on a Mac without them.

So on the commonest Mac the script checks only the file's hash against a manifest it downloaded from the same place, and says so. That catches a damaged download, not a release replaced upstream. The same happens on a Linux system with OpenSSL 1.1 (no `-rawin` before OpenSSL 3), and on Windows, which has no `openssl` at all.

What the stock Mac does have: `/usr/bin/ssh-keygen` (OpenSSH 10.2 there), whose `-Y verify` checks SSH signatures made with Ed25519 keys.

## 2. Decision

Each release carries one more file, **`AgentMux-<version>-envelope.json.sshsig`**: the same envelope, signed by the same key (`k1`), in SSH signature (SSHSIG) form. A system whose `ssh-keygen` has `-Y verify` (OpenSSH 8.0 and later) checks that file with its own tool. No new key, no new trust root, no new service permission; the app's updater is unchanged and ignores the file.

Rejected: a second signature algorithm that LibreSSL can check (ECDSA or RSA: a second key, pinned in the app and both scripts); Ed25519 checked by code written in `sh` or `awk` (hand-made crypto in an installer); a small verifier downloaded first (its trust root would be the script that fetched it).

## 3. The file

The SSHSIG format of OpenSSH (`PROTOCOL.sshsig`). The signer signs this preimage with plain Ed25519 (the same operation as for the `.sig` line, over different bytes):

```
"SSHSIG" || string("agentmux-release") || string("") || string("sha512") || string(SHA-512(envelope bytes))
```

where `string(x)` is a 4-byte big-endian length and the bytes. The file is the armoured blob (`-----BEGIN SSH SIGNATURE-----` ...) holding the version (1), the public key, the namespace, the reserved empty string, the hash name and the signature. Fixed choices: namespace `agentmux-release` (so a signature made with this key for another purpose cannot be replayed as a release signature), hash `sha512`, key id `k1`.

The file is not listed in the manifest (the envelope and its `.sig` are not either).

## 4. Release side (no change to a workflow file)

Everything is in `scripts/release-manifest.mjs`; `.github/workflows/release-core.yml` is not edited.

- **`sign-kms`** (the existing step of the `sign-release` job) already signs the envelope with `aws kms sign` (the signing service's key, raw message, `ED25519_SHA_512`) and writes `AgentMux-<v>-envelope.json.sig`. It now also calls the same service with the same key id and the same role a second time, with the preimage as the message, and writes `AgentMux-<v>-envelope.json.sshsig` next to the `--out` file (the name is derived: `.sig` becomes `.sshsig`; `--sshsig-out` names it explicitly). No new permission is needed: it is the same `kms:Sign` on the same key. The public key it embeds comes from `crates/common/update-keys.json` (the pinned key for `--id`), not from the service, and the signature is checked against that key before the file is written: a signing service whose key is not the pinned one, or that returns garbage, fails the release step with a non-zero exit and leaves no `.sshsig` behind (a file from an earlier run is removed first, and the file is written under a temporary name and renamed).
- **Upload.** The `release-manifest/` folder is already uploaded as one artifact, and the `publish` job copies everything in it into the release (`cp release-manifest/* dist-flat/`), so the new file is published without a list to change. It is attached beside the other two.
- **`verify`** (already run in `sign-release` and again in `publish`, before anything is published) checks the SSH signature when the file is there: with this script's own strict parser (the key inside the file must be the pinned key, the namespace and hash the fixed ones, nothing after the end) and with the real `ssh-keygen -Y verify` on the runner against an allowed-signers file made from the pinned key. For a release at or above the floor (§6) the file must be there. Either failure fails the release before publication. The runner needs the OpenSSH client (the ubuntu runner has it); `--no-ssh-keygen` skips the real tool on a machine that has none (tests only, and the workflow does not pass it).

## 5. Installer side

In `install.sh`, `download_verified` checks the signature in this order:

1. With an OpenSSL 3 (the PATH's, or Homebrew's or MacPorts' in their usual places): the `.sig` line, as before. Nothing else is fetched.
2. Otherwise, when `ssh-keygen -Y` exists (a probe: OpenSSH before 8.0 answers "illegal option") and the release is at or above the floor: it downloads `AgentMux-<v>-envelope.json.sshsig`, builds the allowed-signers file **only** from the keys embedded in the script (`UPDATE_KEYS`; the key carried inside the signature is never trusted), and runs `ssh-keygen -Y verify -f <allowed> -I agentmux-release -n agentmux-release -s <file> < envelope.json`; each allowed-signers line also carries `namespaces="agentmux-release"`, so the key is bound to the namespace in the file as well as by `-n`. Never `check-novalidate`.
3. Otherwise: the size and SHA-256 only, with the plain message ("signature NOT checked: ..., what is checked instead: ..."), as in A.

The output says which check ran: "signature of the release checked: Ed25519, with OpenSSL 3.x" or "... Ed25519 (SSH signature), with OpenSSH_10.2p1". `--require-signature` passes with either and refuses with neither.

Nothing needs to be installed. All the tools used (`ssh-keygen`, `printf`, `base64` or `openssl base64`) are in a stock macOS and a stock Linux.

## 6. No silent downgrade: the floor

`SSHSIG_FLOOR` is the first release that carries the file. It is one value, in `scripts/release-manifest.mjs` and in `install.sh`, and a test fails when the two differ. At or above it:
- the release job refuses a release without the file (§4);
- `install.sh`, on a system that can only check by SSH signature, treats a missing or unreadable `.sshsig` as a failure with a plain message ("this release should carry ..., not installing"), never as a fallback to hashes.

Below the floor, today's behaviour. A system with an OpenSSL 3 never needs the file.

The floor is the first release cut after the change merges, and the release owner gives its value (it is 0.60.31 at the time of writing); the change is not merged until the owner says so.

## 7. Older systems

- macOS: the desktop app and the server package need Apple Silicon, which needs macOS 11 or later. Measured: macOS 26.5.2 (OpenSSH 10.2). From memory, not verified: macOS 11 ships OpenSSH 8.1 and `-Y verify` arrived in OpenSSH 8.0, so every supported Mac has it.
- Linux: OpenSSH 8.0 or later has it (from memory: Ubuntu 20.04 has 8.2, Debian 11 has 8.4). A system with an older one, such as CentOS 7 (7.4), gets the plain hash-only message unless it has an OpenSSL 3.
- Releases before the floor are checked as before.

## 8. `install.ps1`

Today it checks the signature only when an `openssl` that can (OpenSSL 3, `pkeyutl -rawin`) is on the PATH; a stock Windows has none, so it checks hashes and says "signature not checked: this system has no OpenSSL 3". Windows' `ssh-keygen.exe` is part of the OpenSSH Client, an optional Windows feature (Microsoft's documentation lists it among the features to check for in Optional Features and to add with `Add-WindowsCapability`); whether it is present by default on Windows 10 and 11 was not verified here. Proposal, not built in this change: the same order as `install.sh` (an OpenSSL 3, then `ssh-keygen -Y verify` when `ssh-keygen.exe` is found, then the plain hash-only message), with the same floor.

## 9. Tests

- `scripts/release-sshsig.test.mjs` (vitest): the preimage layout; the armour round trip and every refusal of the strict parser (no armour, bad magic, cut short, bytes after the end); a good signature; **one byte changed**; **another namespace** (validly signed for it); **a signature by another key** under the pinned key's name; **a good SSH signature by a key that is not the pinned one**; the floor (required at it, optional and still checked below it); a version that is not exactly digits X.Y.Z is refused rather than compared; the command line through the local-key path (`sign-key --sshsig-out`) and through `sign-kms` with a stand-in for `aws kms sign` (two calls, the same key id, a second message; both files written; a signing service whose key is not the pinned one writes no `.sshsig`); the real `ssh-keygen` accepts what we write and refuses the three bad cases, and **our parser and checker accept a signature that OpenSSH itself made**. `CI` set without `ssh-keygen` fails the file, so the real-tool checks cannot be skipped there.
- `scripts/install/install.test.sh`: a stock-Mac-like system (a LibreSSL that cannot check) with a real `ssh-keygen`: checked, and which check is named; `--require-signature` passes; a changed envelope, another namespace and a good signature by an unpinned key are refused; from the floor on a release without the file is refused and not installed on hashes; below the floor, and with an old OpenSSH (`-Y` unknown), hashes only with the plain message and `--require-signature` refusing; with OpenSSL 3 the `.sig` is used and no `.sshsig` is needed; the two floors are equal.
- The same checks on a real stock Mac and a real Linux host are asked of the platform testers before merge.

## 10. Not in this change

- **A known gap:** when "latest" resolves to a version below the floor, a system that can only check by SSH signature still installs it on hashes (a rollback to a release from before the floor). It is not closed here because a script fetched between this merge and the first release that carries the file would then refuse the then-latest release. It is for a follow-up after that release.
- `install.ps1`; the landing page's wording about verification; the `agentmux-install-update.sh` and the app's updater, which keep checking the `.sig` line.
