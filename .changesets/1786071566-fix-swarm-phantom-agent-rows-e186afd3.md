---
type: patch
---

fix(swarm): stop rendering a phantom "Agent" row for a block that doesn't exist

`buildTree()` added every subagent's `parent_block_id` to the swarm tree's row
list unconditionally, even when no WOS block object exists for that id (a
parent block whose registration never completed, or was pruned while a
subagent record referencing it lingered). That row rendered as a placeholder
named literally "Agent," and any dispatch grouped under it showed as an empty
"No activity yet" entry beside it. `buildTree()` now skips a row entirely when
its block doesn't resolve to a real object, via the new `hasRenderableBlock()`
predicate — distinct from the pre-existing "Agent" fallback for a real block
whose `agentName` meta just hasn't propagated yet, which is unchanged.
