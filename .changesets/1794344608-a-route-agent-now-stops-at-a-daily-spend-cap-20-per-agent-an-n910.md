---
type: patch
---

A route agent now stops at a daily spend cap: $20 per agent and $100 per route (the day is kept across restarts), the gateway refuses with a 402, its leader is told at 50%, the pane warns at 80% and the turn ends at 100%; a leader raises or resets the cap with Agents update spend_cap_usd
