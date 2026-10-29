---
type: minor
---

Container agents get every srv route a native agent has (Presence, Vault, Browser and UI, Tasks, Schedule, Agents, Widget, RegisterDevServer and the rest) except the ones that run commands on the host, read host files outside their mounts or are the app's own plumbing, and a refused route says why and what to do instead; a container agent's browser defaults to its own Incognito jar, never the user's Personal profile
