---
type: patch
---

Jekts sent from this install keep arriving as verified after the cloud relay or the signed-in account changes. Agent signing keys are now recorded as published per relay and account, so the install publishes them again to a key directory that has never seen them (including once, automatically, after this update). Before, keys were only ever published once, so after such a change they arrived unverified.
