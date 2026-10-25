---
type: patch
---

The app now carries the public key that release manifests are signed with (nothing checks it yet), and the release job refuses to sign a commit that is not on main
