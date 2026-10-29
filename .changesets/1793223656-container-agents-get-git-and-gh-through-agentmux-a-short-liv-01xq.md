---
type: minor
---

Container agents get git and gh through AgentMux: a short-lived GitHub token from the host's gh-agent (or the connected account) for the repository they work on, the push scan and commit trailers mounted read-only, and no token written into the container.
