# Retro — Claude v2.1.x in-app login: the spawn can't open the browser (2026-06-23)

## TL;DR
We spent the session trying to make Claude's in-app login work by driving the CLI under
our host PTY — first scraping the OAuth URL, then pivoting to `claude setup-token`
(§5.1 of `SPEC_HOST_CLI_LOGIN_CAPTURE`). Two real capture attempts (agents "Marks", "Lazo")
**both hung with no browser and no token output.** Web research settled it: Claude Code
**v2.1.x** opens the OS default browser via a local OAuth callback — but **in any spawned /
piped / tmux / headless context the flow hangs silently** (no browser). Our host→PTY spawn
*is* such a context, so neither `auth login` nor `setup-token` can complete in-app.
**Upstream's own recommendation (issue #7100) is exactly our seed-from-global path:** run
`setup-token` once on a machine with a browser, then seed those credentials into the
headless environment. **Decision: make seed-from-global the PRIMARY Claude auth path; stop
trying to drive OAuth inside AgentMux's spawn.**

## Timeline
- §4 instrumentation + §5.5 seed-from-global shipped earlier (#1613).
- This session: chased §5.1 — flipped Claude's `authLoginCommand` to `setup-token`,
  instrumented `run_cli_login_pty` (token redaction in logs, token-line detection,
  post-login `.credentials.json` check), built two capture portables.
- Capture 1 ("Marks") + Capture 2 ("Lazo"): `run_cli_login: spawned (PTY)` →
  `no auth URL captured within 15s`, **zero `[login-pty]` lines** (the TUI redraws with no
  newlines → `read_line` blocks), **no browser opened**, no token.
- User (correctly) pushed back on "it can't open a browser." Researched it.

## What's actually true (evidence)
- The OAuth flow **opens the default browser** to `console.anthropic.com`, runs a localhost
  callback, exchanges the code for access+refresh tokens. ([authentication docs](https://code.claude.com/docs/en/authentication))
- **But headless/piped/tmux/spawned contexts hang silently** — `/login` isn't available in
  `-p` mode; the browser flow "hangs or errors" in containers. ([headless docs](https://code.claude.com/docs/en/headless),
  [login-not-working](https://www.remoteopenclaw.com/blog/claude-code-login-not-working-fix))
- Upstream's documented remedy = run `setup-token` on a browser machine, **seed the creds**
  into the headless env. ([#7100](https://github.com/anthropics/claude-code/issues/7100))
- "It worked before" = a *real terminal* (the user's global `setup-token`) gets the browser;
  older CLI versions printed a scrapeable URL. The v2.1.x **spawned** case is the dead end.

## Why we chased the wrong thing
- `setup-token` looked like a clean "headless contract" (prints a token), so it read as the
  fix. But it shares the **same browser front-end** — the token only prints *after* an OAuth
  that can't start under our spawn. The headless contract is "run it where a browser works,"
  not "we can run it for you."

## Lessons
1. **Don't assert mechanism without evidence.** "The spawn can't open a browser" was stated
   too absolutely; the truth was nuanced (opens in real terminals, hangs when spawned). The
   user's pushback → web research → correct, defensible model. Research *before* the strong
   claim, not after.
2. **The CLI's auth UX is not ours to drive.** Three approaches (URL-scrape, setup-token
   capture, paste-code) all founder on the same rock: the v2.1.x login is a self-driving TUI
   that won't open a browser when spawned. Stop fighting it.
3. **Seed-from-global is the robust, upstream-blessed path** — and it works *now* (a valid
   global login seeds the isolated dir, `refreshToken` keeps it alive). Make it primary, not
   a 401 fallback.
4. **Auth × lifecycle interaction (bonus orphan-leak):** a pending login PTY arms a 6-min
   reap task that counts as "live work" and can **block the host's clean exit** — closing all
   windows left a 9-proc orphan tree even though the #1676 quit-chain fired
   (`0 visible — quitting message loop`). Recorded in the lifecycle consolidation notes; the
   robust machine must cancel the login reap on quit.

## Salvage (kept)
- `redact_secrets()` + token-detection in `run_cli_login_pty` — a real security fix
  (`setup-token` would otherwise log a live `CLAUDE_CODE_OAUTH_TOKEN`); keep it regardless.
- The slice-1 PTY capture proved its worth: it's how we confirmed "no output, no browser."

## Next (the robust machine)
- Make **seed-from-global the primary** Claude auth path: on launch/create of a Claude agent
  that lacks a valid isolated credential, if a valid global login exists, offer/auto a
  one-click "Use my existing login" up-front (not only on the 401 row).
- Keep `auth login`/`setup-token` only as the "no global login yet → go authenticate in a
  real terminal, then seed" instruction.
- Cancel the login PTY reap on host quit (close the orphan-leak vector).

Sources: [authentication](https://code.claude.com/docs/en/authentication) ·
[headless](https://code.claude.com/docs/en/headless) ·
[#7100](https://github.com/anthropics/claude-code/issues/7100) ·
[login-not-working](https://www.remoteopenclaw.com/blog/claude-code-login-not-working-fix)
