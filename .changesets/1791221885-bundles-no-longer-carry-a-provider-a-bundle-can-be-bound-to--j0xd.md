---
type: minor
---

Bundles no longer carry a provider: a bundle can be bound to any agent, and the agent's own provider decides which CLI it runs. An agent's provider is now fixed once set (fork the agent to run it on another provider), and an upgrade migration first gives every agent the provider it already runs with, so no agent changes CLI.
