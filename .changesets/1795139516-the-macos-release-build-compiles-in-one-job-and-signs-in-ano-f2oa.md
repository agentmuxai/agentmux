---
type: patch
---

The macOS release build compiles in one job and signs in another, and the app inside the DMG now carries its own notarization ticket, so a Mac that is offline can open a fresh download. A notarization that Apple does not accept now fails the macOS leg of a release instead of shipping a signed-only DMG: the release has no macOS DMG until that is fixed, and the leg can be re-run
