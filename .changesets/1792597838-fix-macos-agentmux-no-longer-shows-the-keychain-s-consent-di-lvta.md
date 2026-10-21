---
type: patch
---

fix(macos): AgentMux no longer shows the keychain's consent dialog on its own; a MuxBus sign-in saved by v0.59.15 or older asks you to sign in again instead, and saving replaces old entries so later upgrades never ask
