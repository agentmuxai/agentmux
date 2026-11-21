# Spec: the upgrade floor is carried in the release's signed manifest

**Task #47853** (spec), build #47854. Winny@narko, 2026-11-20. Uncommitted: it ships in the build's first PR. Requirements and tests only.

## 1. What the build guarantees

1. **The floor is signed.** Every release's manifest (covered by the signed envelope through `manifest_sha256`) states the oldest version that may upgrade straight to that release, taken from the repository's `UPGRADE_FLOOR` at the release's tag. An update check that reads a release from its signed files learns the floor from them, not from GitHub's release notes or a raw file, which are not signed.
2. **One optional field, no format change.** The field is an optional object `upgrade` at the top of the manifest, in `format` 1. A reader that does not know it ignores it and verifies the release exactly as before (see 2). `format` is not raised: every released app refuses a format it does not know, so a format 2 would stop every installed app from seeing any release.
3. **The floor wins over unsigned sources, and never lowers the build's.** When the manifest carries a valid floor, the floor in force is the higher of that floor and the one built into the running app (`BUILD_FLOOR`). The release notes' floor line and the raw `UPGRADE_FLOOR` file are not consulted for that release. When the manifest carries none, behaviour is today's, unchanged: the higher of `BUILD_FLOOR` and what the notes or the floor file at the tag say.
4. **One verifier for the check and the download.** The check and the download read the floor through the same verified manifest (the verifier of #7564: same keys, same signature rules, a second signature included). A check cannot offer an upgrade the download would refuse for the floor.
5. **A floor above the running version blocks the direct upgrade and says why.** The check reports the release as not directly installable from this version, names the floor, and offers that version first, as the notes' floor line does today. The download refuses with the same reason.
6. **A bad floor never makes things less safe.** A present but unusable `upgrade.floor` (not a plain version, or above the release's own version) is logged once per tag and treated as no signed floor: the floor in force is then today's (guarantee 3, last sentence). It does not by itself refuse the download or fail the manifest's verification: the files were still verified, and the pipeline's own check (guarantee 7) means a signed release should never carry one.
7. **The release script writes it, and refuses to write nonsense.** `scripts/release-manifest.mjs build` reads `UPGRADE_FLOOR` (a plain `x.y.z` and a newline, the file the notes' floor line is also written from) and writes `upgrade.floor`. It fails the build, so nothing is signed, when the file is missing, is not a plain version, or is above the version being released. The notes' floor line and the signed floor come from the same file and so cannot differ.
8. **Verified by the script too.** `release-manifest.mjs verify` checks `upgrade` with the same rules as the app (guarantees 6 and 7). Other readers of the manifest (the install scripts, the landing's mirror check) ignore the field and still verify.

## 2. The compatibility rule (the floor and the rollback field are one change)

Both the floor and the shared-data field of the fleet-rollout spec (#11442, `shared_store_changed`) live in the same optional object, `upgrade`, written by the same script step and read by the same code. One rule covers both, and any later field in it:

- **Shape.** `upgrade` is a JSON object; `upgrade.floor` is a string (a plain `x.y.z`); `upgrade.shared_store_changed` is a boolean (`true`: the release changes a shared store's schema compared with the previous release; `false` or absent: no warning).
- **Additive and optional.** A new field is added to `upgrade` and never changes `format`. Fields inside `upgrade` that a reader does not know are ignored.
- **Unknown to the reader: ignored.** An app, script or service that does not know a field behaves as it did before the field existed. This is why the manifest parsers in the app and the scripts must not reject unknown fields (tests, section 4).
- **Known but unusable: that field alone is treated as absent**, logged once, and each field says what absent means for it. For the floor: today's behaviour. For `shared_store_changed`: absent means "not known" and is treated as no warning (the field only decides whether a banner warns that a return from this release loses data written after it starts; the rollback's safety comes from the snapshot, not from the field, so an absent or unreadable one cannot make anything unsafe).
- **A field is never trusted from anywhere but the verified manifest.** Nothing reads `upgrade` from a manifest that did not verify.

## 3. What does not change

- Envelope: unchanged (it stays small, and the scripts that read it with plain text tools are not touched).
- The signature, the second signature, the keys, the files and their hashes.
- Today's apps: they ignore `upgrade` and keep the floor they get from the notes and the file at the tag. The signed floor protects an app only from the version that reads it on.
- No release is cut for this work.

## 4. Tests

Fixtures are shared: one set of manifests and envelopes signed with a test key, generated by `release-manifest.mjs` and read by the Rust tests as well as the script's, so both sides check the same bytes.

1. **The script's output verifies with the app's verifier**: a manifest with `upgrade.floor` written by `build` passes `verify_release` and gives the floor.
2. **A signed floor above the running build blocks the direct upgrade and says why**: the check reports it not directly installable and names the floor and the version to install first; the download refuses with the same reason. At or above the floor: as before.
3. **An app that does not know the field ignores it**: the pre-change `Manifest` and `Envelope` shapes (kept in the test as the frozen old struct), the JS `verify` of the previous release of the script, `install.sh`'s and `install.ps1`'s manifest readers, and the landing's `checkRelease` all verify a manifest with `upgrade` (including `shared_store_changed` and an unknown field inside it) and read the same files.
4. **The floor in force is the higher of the signed floor and `BUILD_FLOOR`**: a signed floor below the build's does not lower it; above it, it raises it.
5. **Signed floor present: the unsigned sources are not consulted**: with a notes floor line or a floor file naming a different floor, the signed one is used (a spy on the floor-file fetch sees no request).
6. **No signed floor: today's behaviour**: a manifest without `upgrade` gives the same answers as before this change (the existing tests pass unchanged).
7. **A bad floor** (not a version, empty, a number, above the release's own version): the files still verify, the floor in force is today's, and it is logged once per tag.
8. **The script refuses to sign nonsense**: `build` fails with no files written when `UPGRADE_FLOOR` is missing, malformed, or above the release version.
9. **The notes' floor line and the signed floor are the same value**: a test over the release-notes script and `build` for one `UPGRADE_FLOOR`.
10. **The envelope is unchanged**: its bytes for a given input are the same before and after the change.
11. **`shared_store_changed`** is read as true, false or absent (absent and a non-boolean both mean no warning, the latter logged once), and never changes the floor or whether a file is offered.
12. **A tampered floor is refused**: changing the manifest's floor changes its hash, so the envelope's signature no longer covers it (`verify_release` fails).

## 5. Who changes what (for the build)

- `crates/common/src/update_manifest.rs`: the optional `upgrade` object, its validation, the floor accessor (a verified-manifest result carries the floor or why there is none).
- `crates/srv-core/src/backend/update/` (`signed_source.rs`, `check.rs`, the download check): use the manifest's floor where the manifest is read; no floor-file fetch when it is present.
- `scripts/release-manifest.mjs` (build and verify) and its tests. The release workflow needs no change if the script reads `UPGRADE_FLOOR` itself; if it does, that diff comes to the release owner before merge, as the script does (release-signing path).

## 6. Decisions for the reviewer

1. The floor goes in the **manifest**, not the envelope (the envelope stays minimal and is read by plain-text tools; the manifest is covered by the signature through its hash). Recommended.
2. **Format 1 with an optional object**, not format 2 (a format bump locks out every installed app). Recommended.
3. **Floor in force = higher of signed and built-in**, signed present means the unsigned sources are not read. Recommended.
4. **A bad signed floor falls back to today's behaviour**, logged, and does not refuse the download by itself. The alternative, to refuse the direct upgrade when the floor is unreadable, is safer against a pipeline bug but can stop all updates until a fixed release; the pipeline's refusal to sign a bad floor (guarantee 7) makes that case a bug, so the fallback is recommended.
5. The `upgrade` object also carries `shared_store_changed` (the rollout spec's field, set by its release-pipeline check that compares the shared stores' schema constants with the previous release) under the compatibility rule of section 2. This spec fixes its place, type and what absent means; how the check computes it and how the banner uses it are that spec's. The release script's `build` takes the check's result as an input and writes it; it does not compute it.
