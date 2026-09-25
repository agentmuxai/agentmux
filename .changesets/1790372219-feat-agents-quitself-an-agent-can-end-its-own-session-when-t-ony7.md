---
type: patch
---

feat(agents): QuitSelf — an agent can end its own session when the user asked it to in this turn; srv checks the turn was the user's and the quote is theirs; anything else warns the user, who has 15 s to keep the agent running (banner, notification, falling chime)
