---
type: patch
---

Update the HTTP client (reqwest) from 0.12 to 0.13: outbound HTTPS now checks server certificates against the operating system's trusted roots instead of a bundled list, on the same ring crypto
