---
type: patch
---

fix(login): widen the login PTY so the OAuth URL never hard-wraps, and restyle the in-app login card

Two follow-ups to #2429/#2436:

1. **The `client_id` truncation bug wasn't actually fixed by the earlier OSC-8 priority change.** A live capture showed the Claude CLI doesn't emit an OSC-8 hyperlink for this URL at all (only an OSC-0 window-title sequence) — so `extract_url()`'s OSC-8 preference had nothing to prefer and kept falling through to the plain-text line, which the CLI's own renderer hard-wraps at the PTY's 80-column width, truncating the URL mid-query-string before `client_id` ever appears. The real fix: widen the login PTY to 4096 columns (both the Windows `PtySize` and Unix `winsize` spawn paths) so the CLI's line-wrapping never kicks in. Confirmed live: the captured URL is no longer split across `[login-pty]` log lines, and login completes successfully end-to-end. The OSC-8 preference is left in place as defense-in-depth for CLIs that do emit one.

2. **Restyled the login card in the agent pane** to match `AgentQuestionPanel`'s look: bordered/shadowed card chrome (it previously rendered as bare, unbordered text — every other caller of `InAppLoginPanel` gets its box from a wrapping `Modal`, which this bottom-docked context doesn't have), each step ("Authorize in your browser" / "Paste the authorization code") boxed as a distinct section, and the backup URL now renders as a fully-visible wrapped block instead of a single-line ellipsis-truncated one.
