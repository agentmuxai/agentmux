---
type: minor
---

Container agents now have CPU and memory limits (half of Docker's memory and all but a quarter of its cores by default), set per agent, in Settings or with Agents set_container_limits, applied to running containers without restarting them; a process killed for going over its memory limit now says so in the pane
