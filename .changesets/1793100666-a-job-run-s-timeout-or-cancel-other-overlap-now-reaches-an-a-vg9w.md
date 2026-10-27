---
type: patch
---

A Job run's timeout or cancel_other overlap now reaches an agent in another AgentMux on the same computer: the AgentMux that follows the run closes it and interrupts the agent's turn, instead of the run being left to go on
