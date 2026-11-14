---
type: patch
---

Core tests: card_target, native_drift, upgrade_notice and durable_ssh tests build the store, file store, event bus and broker they need through a shared core fixture instead of server::tests::test_state (prep for the srv-core split)
