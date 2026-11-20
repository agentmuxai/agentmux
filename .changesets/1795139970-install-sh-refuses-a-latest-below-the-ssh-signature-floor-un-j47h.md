---
type: patch
---

install.sh refuses a "latest" below the SSH-signature floor unless its Ed25519 signature is checked; the signature parser refuses length fields that run past the end; a test keeps the tests-only ssh-keygen switch out of the workflows
