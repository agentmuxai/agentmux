---
type: patch
---

feat(menu): hamburger order — Settings, Command Palette, DevTools, Online Docs

Follow-up tweak to PR #936's hamburger menu. The bottom group is reordered
to **Settings · Command Palette · DevTools · Online Docs**, and the
"Documentation" item is renamed to **"Online Docs"** (its action — open
`https://docs.agentmux.ai` in the browser — is unchanged).

Frontend-only; one block in `tabbar.tsx`.
