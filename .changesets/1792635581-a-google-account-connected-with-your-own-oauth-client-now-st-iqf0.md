---
type: minor
---

A Google account connected with your own OAuth client now stays connected: AgentMux asks Google for offline access and refreshes the token itself (marking the account as needing a reconnect when Google drops the grant), records the account's email and the permissions Google granted, can add permissions to an account already connected, revokes the grant at Google when you remove the account, and remembers your OAuth client so you enter it once. An OAuth-connected GitHub account now gives agents its token instead of the stored JSON.
