---
type: patch
---

Tests no longer announce a fake LAN peer on the real network: a test build's discovery controller never opens an mDNS daemon, so a developer's test run can't add an entry to the peer list of an AgentMux on the same machine
