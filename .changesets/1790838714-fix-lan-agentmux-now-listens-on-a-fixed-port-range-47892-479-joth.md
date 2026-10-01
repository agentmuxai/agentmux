---
type: patch
---

fix(lan): AgentMux now listens on a fixed port range (29700–29799, the next free pair per instance) instead of random ports, so one firewall rule can cover every build and update — the first step towards LAN working on a fresh install without manual firewall steps
