---
type: patch
---

The dev proxy reaches a container agent's dev server on Docker Desktop (a relay container on the agents network with one loopback port), passes WebSocket upgrades through so hot reload works, and the container dev-server route is checked against a real daemon
