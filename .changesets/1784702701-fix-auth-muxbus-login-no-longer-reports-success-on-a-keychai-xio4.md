---
type: patch
---

fix(auth): muxbus.login no longer reports success on a keychain save failure; corrupted token blob surfaces as an error, not a logout
