---
type: minor
---

feat(trust-center): secure API-key storage backend — validate keys against the live service, store them in the OS keychain (never plaintext in the DB), and expose them via the new account.key.verify RPC
