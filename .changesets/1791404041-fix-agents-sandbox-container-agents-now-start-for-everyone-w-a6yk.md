---
type: patch
---

fix(agents): sandbox (container) agents now start for everyone with Docker. The default image is public and carries no Claude Code; AgentMux installs Claude Code in the container on first start and shows the progress in the pane. A failed image download now says what happened and offers running on this computer instead, and the New Agent dialog stops preselecting a sandbox whose image cannot be downloaded.
