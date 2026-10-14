---
type: patch
---

After an account switch the agent now reliably catches up: AgentMux tells the pane when the new session started with nobody talking to it, instead of the pane guessing from a turn state that the switch can leave stale. A deferred catch-up also fires when the pane goes quiet without a turn-end, and the memory-delivery log now says why a session start delivered nothing.
