---
type: patch
---

container agents: on a machine with gh-agent, GitHub access is narrowed to one repository and one use (git writes contents, gh does not), gh pr merge is done by AgentMux, and raw API writes that change refs, tags, releases or files say to use git
