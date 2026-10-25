---
type: minor
---

Jobs get a definition: scheduled jobs are stored as job definitions (trigger, action, policy), cron triggers take an IANA time zone with daylight-saving handling and a fixed per-job jitter, and the Schedule tool can set tz and policy; jobs are served on /agentmux/jobs with /agentmux/cron kept as an alias
