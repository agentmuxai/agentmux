---
type: patch
---

fix(agent): harden the AskUserQuestion dead-air fallback (codex review on #1536). Snapshot stdout activity *before* sending the answer (not after), and count *every* stdout frame — including control frames the reader skips for health monitoring — via a dedicated `stdout_seq` counter. A resume whose first activity is a tool-permission round-trip is no longer mistaken for "no activity" (which would have spuriously re-delivered the answer). Happy path unchanged.
