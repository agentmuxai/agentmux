---
type: patch
---

23 more RPC stubs generated, including the special cases: list commands that now send {} when called without data, raw-JSON commands keeping their precise types, and requests whose defaulted fields the generated type makes optional (proven by srv tests)
