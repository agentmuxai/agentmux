---
type: patch
---

Vault: the Vault's merge rule is now an adapter of the sync layer: an edit on top of what the other machine holds is applied, two changes made apart keep both with the later current, an edit beats a delete it did not see, and a rotation in progress is not overwritten.
