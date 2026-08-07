---
type: patch
---

fix(login): make tier-1 PTY capture actually work end-to-end

Fixes #2429, in three parts:

1. **Root cause — the CLI never printed anything.** Claude Code, on detecting a real TTY, immediately sends a cursor-position query (`ESC[6n`) and blocks waiting for a reply before printing anything — nothing was ever answering it. `run_cli_login_pty` now transparently answers this query the moment it appears in the stream, unblocking the CLI. Confirmed live: URL capture now takes ~450ms instead of timing out at 15s with zero bytes seen.

2. **Follow-up — the captured URL was missing `client_id`.** Once capture started working, `extract_url()` preferred the CLI's plain-text fallback line, which gets word-wrapped by the PTY's 80-column width mid-query-string. The OSC-8 hyperlink payload (immune to that wrapping, since it's inside an escape sequence, not printed text) carries the complete URL the whole time — `extract_url()` now prefers it.

3. **Follow-up — the login UI rendered pinned to the top of the pane.** It was rendered inside `AgentDocumentView`'s scrollable header slot. Extracted into `AgentAuthPanel`, now rendered by `agent-view.tsx` as a flex sibling after the scroll region — same bottom-docked slot band as `AgentDecisionPanel`/`AgentQuestionPanel`.
