---
type: minor
---

AgentMux now checks for a newer release by itself (a minute after start, then every few hours), downloads and verifies it in the background, and the status bar offers Restart to update. Nothing installs until you restart. Turn it off with update:auto: false, or keep the check and skip the download with update:autodownload: false.
