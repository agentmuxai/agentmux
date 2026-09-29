---
type: patch
---

Signing in to AgentMux Cloud again now republishes this install's agent signing keys, so jekts sent from here keep arriving as verified after the cloud's key directory changes (a new relay or a different account). Before, keys were only ever published once, so after such a change they arrived unverified.
