---
type: patch
---

A worker an agent starts is now a new agent, not a fork: Agents fork is renamed Agents spawn, the worker no longer shows among its leader's forks, and it reports back through AgentMux's Message tool. It runs on its leader's account, or on another signed-in one when the leader's is out of quota or signed out. AgentMux also recognizes Claude's weekly and session limits as the plan's usage limit
