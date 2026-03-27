# macOS Code Signing & Notarization

This document covers how to sign and notarize AgentMux DMGs for direct distribution.

Credential details (Apple ID, Team ID, keychain profile, certificate name) are kept in the
private [agentmux-builder](https://github.com/agentmuxai/agentmux-builder) repo.

---

## Prerequisites

- A **Developer ID Application** certificate installed in your login Keychain
- An **app-specific password** stored as a `notarytool` Keychain profile (see agentmux-builder docs)
- `xcrun notarytool` — ships with Xcode Command Line Tools

---

## One-Time Setup: Store Notarization Credentials

Run once to save credentials to the Keychain:

```bash
xcrun notarytool store-credentials "notarytool" \
  --apple-id "<your-apple-id>" \
  --password "<app-specific-password>" \
  --team-id "<your-team-id>"
```

App-specific passwords are generated at [appleid.apple.com](https://appleid.apple.com) →
Sign-In & Security → App-Specific Passwords. Format: `xxxx-xxxx-xxxx-xxxx`.

Once stored, reference credentials by profile name in all subsequent calls — the raw
password is never needed again.

---

## Signing Workflow

Run after `task package:macos` has built the `.app` and initial DMG.

### 1. Sign all binaries inside the .app

The `.app` bundle contains multiple Mach-O executables. Each must be individually signed
with `--options runtime` (hardened runtime) before notarization will accept them.

```bash
APP=target/release/bundle/macos/AgentMux.app
VERSION=$(node -p "require('./package.json').version")
CERT="<your-developer-id-cert-name>"   # e.g. "Developer ID Application: Name (TEAMID)"

# Sign resource binaries first
codesign --force --options runtime --sign "$CERT" \
  "$APP/Contents/Resources/binaries/bin/wsh-${VERSION}-darwin.arm64"

# Sign MacOS binaries
codesign --force --options runtime --sign "$CERT" \
  "$APP/Contents/MacOS/agentmuxsrv-rs"

codesign --force --options runtime --sign "$CERT" \
  "$APP/Contents/MacOS/wsh"

# Re-seal the .app bundle
codesign --deep --force --options runtime --sign "$CERT" "$APP"
```

> **Why sign individually before `--deep`?** `--deep` seals nested content — signing
> individual binaries first ensures their signatures are included in the bundle seal.

### 2. Re-create and sign the DMG

The DMG must be rebuilt from the signed `.app`. Signing the pre-existing DMG won't work
because it was created before the binaries were signed.

```bash
OUT=target/release/bundle/macos/AgentMux_${VERSION}_aarch64.dmg

hdiutil create -volname "AgentMux-${VERSION}" -srcfolder "$APP" -ov -format UDZO "$OUT"
codesign --force --sign "$CERT" "$OUT"
```

### 3. Notarize

```bash
xcrun notarytool submit "$OUT" --keychain-profile "notarytool" --wait
```

Expected output when successful:
```
status: Accepted
```

If status is `Invalid`, fetch the rejection log:
```bash
xcrun notarytool log <submission-id> --keychain-profile "notarytool"
```

Common rejection reasons:
| Error | Fix |
|-------|-----|
| `The binary is not signed with a valid Developer ID certificate` | Sign each binary individually (step 1) |
| `The signature does not include a secure timestamp` | Add `--options runtime` to codesign |
| `The executable does not have the hardened runtime enabled` | Add `--options runtime` to codesign |

### 4. Staple

Attach the notarization ticket to the DMG so Gatekeeper works offline:

```bash
xcrun stapler staple "$OUT"
# Expected: "The staple and validate action worked!"
```

### 5. Verify

```bash
spctl --assess --type open --context context:primary-signature -v "$OUT"
# Expected: "source=Notarized Developer ID"
```

---

## Upload to GitHub Release

```bash
cp "$OUT" ~/Desktop/
gh release upload "v${VERSION}" ~/Desktop/AgentMux_${VERSION}_aarch64.dmg --clobber
```

---

## Full Script

```bash
#!/bin/bash
set -e

APP=target/release/bundle/macos/AgentMux.app
VERSION=$(node -p "require('./package.json').version")
OUT=target/release/bundle/macos/AgentMux_${VERSION}_aarch64.dmg
CERT="<your-developer-id-cert-name>"

echo "==> Signing binaries..."
codesign --force --options runtime --sign "$CERT" \
  "$APP/Contents/Resources/binaries/bin/wsh-${VERSION}-darwin.arm64"
codesign --force --options runtime --sign "$CERT" \
  "$APP/Contents/MacOS/agentmuxsrv-rs"
codesign --force --options runtime --sign "$CERT" \
  "$APP/Contents/MacOS/wsh"
codesign --deep --force --options runtime --sign "$CERT" "$APP"

echo "==> Creating and signing DMG..."
hdiutil create -volname "AgentMux-${VERSION}" -srcfolder "$APP" -ov -format UDZO "$OUT"
codesign --force --sign "$CERT" "$OUT"

echo "==> Notarizing..."
xcrun notarytool submit "$OUT" --keychain-profile "notarytool" --wait

echo "==> Stapling..."
xcrun stapler staple "$OUT"

echo "==> Done: $OUT"
```

---

## CI / Automated Signing

The [agentmux-builder](https://github.com/agentmuxai/agentmux-builder) workflow handles
signing automatically using GitHub Actions secrets. See `agentmux-builder` for the secret
names required and how the CI pipeline maps to these steps.

---

## Notes

- Notarization typically takes 1–3 minutes. If it stalls >15 min, check
  [Apple's developer status page](https://developer.apple.com/system-status/).
- This flow is for **direct distribution** (outside the App Store). Mac App Store
  distribution requires a different certificate type, provisioning profiles, and App
  Sandbox entitlements — see `docs/macos-appstore.md` if that work is ever pursued.
