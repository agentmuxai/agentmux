---
type: patch
---

A message to an agent whose account is out of quota is now held and delivered when it can work again (answered "HELD for X (usage_limit, until …)"), instead of being lost in a turn that fails at once; where it can't be held it is refused with the reason, and a Swarm broadcast says so in its result.
