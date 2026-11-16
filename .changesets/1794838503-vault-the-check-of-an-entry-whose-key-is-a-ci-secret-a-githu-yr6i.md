---
type: patch
---

Vault: the check of an entry whose key is a CI secret (a GitHub Actions secret of a repository or an environment) says whether the secret is there and when it last changed, read with the GitHub CLI signed in on the computer. A container agent's check reads it only for an entry the person answered Always for; every read is a line in Audit
