# Retro — `/login` has a working browser-opener sitting unused right next to it

**Date:** 2026-07-20
**Trigger:** Live repro — AgentA's pane hit Claude Code CLI's "Not logged in.
Please run `/login`." banner. Running `/login` visibly did nothing: no
browser opened, no URL appeared, no state changed. Separately, the shared
provider auth dir (`~/.agentmux/shared/providers/claude/`) turned out to
have been silently logged out (empty tokens) 47 minutes earlier, with zero
`muxlog auth` evidence of what caused it — see §4.
**Audience:** anyone touching `run_cli_login`/`run_cli_login_pty`,
`open_login_terminal`, `frontend/app/view/agent/commands/global/login.ts`,
or `force-login.ts`. Read this before changing how a login attempt picks
its spawn strategy.
**Follow-up:** the fix below (§5) initially covered `/login` and "Login
Again" only. Live verification found a THIRD, independent implementation
of the same "spawn login, wait for URL" pattern in the gated launch flow —
the one the "Retry Login" button actually triggers — that bypassed both
the fix and the diagnosis here. See
`docs/retro/retro-login-three-code-paths-2026-07-20.md` for that gap and
its fix; read both before touching login flows.

---

## 1. The question this retro answers

**"Why does AgentMux have such a hard time opening the browser to log in?"**

It doesn't, actually — the OS-level opener works fine and is exercised
constantly for ordinary links (`open_url_in_default_browser`,
`agentmux-cef/src/commands/platform.rs:1518-1560`: `rundll32.exe
url.dll,FileProtocolHandler` on Windows, `open` on macOS, `xdg-open` on
Linux — chosen specifically to avoid `explorer.exe`/`cmd /C start`
injection and file-manager-window bugs, per the comment at `:1531-1538`).

The actual problem is one step upstream: **AgentMux never gets a URL to
hand that opener.** `/login` and "Login Again" both call
`forceProviderLogin` → the CEF `run_cli_login`/`run_cli_login_pty` IPC path
(`ipc.rs:399`, `platform.rs:417-640` / `880-1131`), which spawns the Claude
CLI **headless** — piped stdio (`Stdio::piped()`, `platform.rs:504-514`) or
a PTY with no attached visible console — and then scans its output for an
`https://` URL (`extract_url`, `platform.rs:1164-1242`).

For Claude Code v2.1.x, that scan finds nothing, ever, in any headless
context. Per the prior investigation this retro builds on
(`docs/retro/retro-claude-v2-1-auth-spawn-2026-06-23.md`,
`docs/specs/SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md:56-67`): the CLI's
OAuth flow opens the browser **itself**, directly, via its own OS calls —
it does not print a URL for a wrapper process to scrape. In a piped/PTY/
detached spawn there is no attached console for the CLI's own browser-open
call to succeed from, so the flow just hangs or silently no-ops. `/login`
"doing nothing" is that CLI-internal browser-open call failing silently,
not a missing feature in AgentMux's opener.

**The fix already exists in this codebase and is not used by default.**
`open_login_terminal` (`platform.rs:1274-1345`) spawns the exact same login
command with `CREATE_NEW_CONSOLE` — a real, visible console window — whose
own doc comment states the diagnosis outright:

> "Spawn the CLI login command in a NEW visible console window so the OS
> can open a browser (the piped/PTY paths used by `run_cli_login` are
> headless and block the browser from launching — confirmed for Claude
> v2.1.x)."

This path works. It is wired to a separate **"Login via terminal"** button
(`useAgentControllerStatus.ts:367-434`, `cef-api.ts:687-688`) that most
users never discover, and it is **Windows-only** —
`platform.rs:1337-1342` returns `Err("open_login_terminal: not yet
implemented on this platform")` on macOS/Linux.

`/login`'s own handler (`frontend/app/view/agent/commands/global/login.ts:44-59`)
already knows this — when `forceProviderLogin` returns `"no-url"`, the error
message it shows is literally:

> "/login: the CLI didn't produce a login URL, so no browser was opened.
> Use the "Login via terminal" or "Use existing login" actions instead."

So the code contains its own correct answer and still doesn't act on it. A
human has to read the error, know which of two other buttons to click, and
retry manually. That hand-off — not the browser-opening mechanism itself —
is "why AgentMux has such a hard time."

## 2. Two more contributing gaps, same shape

- **AgentMux's own default pane spawn is unconditionally headless**, for a
  reason that has nothing to do with auth: `CREATE_NO_WINDOW` + fully piped
  stdio on the "persistent" controller (`agentmux-srv/src/backend/blockcontroller/persistent.rs:674-690`)
  and the subprocess controller (`.../subprocess.rs:393-404`) exists to fix
  a **Windows Terminal window leak**
  (`docs/retro/retro-windows-terminal-window-leak-2026-06-21.md`). That fix
  is correct for its own problem — but its side effect is that every normal
  agent pane's long-running Claude process is permanently in exactly the
  spawn shape (§1) that defeats interactive OAuth. There's no cross-link
  between that retro and this login-capture problem anywhere in the repo;
  each was fixed in isolation.
- **`/login`'s no-url failure used to be silent**, not just unhelpful — the
  explicit error message quoted above (`login.ts:51-59`) and the equivalent
  branch in `useAgentControllerStatus.ts:299-306` were both added by
  `docs/retro/retro-agent-auth-relogin-noop-2026-07-01.md` specifically to
  stop the *"no error, no browser, nothing happens"* failure mode from
  looking like success. That retro's fix made the dead end **visible**. It
  did not make the dead end **navigable** — the user still has to
  self-serve the correct button.

Three retros (`2026-06-23`, `2026-07-01`, this one) have now separately
diagnosed pieces of the same underlying gap: *the piped/headless path is
structurally incapable of completing Claude v2.1.x's login, and the working
alternative is not the default.* Each fix narrowed the symptom without
closing the gap.

## 3. Why this specific incident additionally involved a fully dead credential file

Independent of §1–2, the *reason* AgentA's pane was logged out at all: the
shared, account-wide `~/.agentmux/shared/providers/claude/.credentials.json`
had `accessToken:""`/`refreshToken:""`/`expiresAt:0` (confirmed on disk,
mtime 07:43 today), while the `.agentmux-cred-seeded` sentinel
(`cli_handlers.rs:313`, created 2026-06-23) was already present. Per the
comment at `cli_handlers.rs:305-310`, that sentinel exists specifically "so
a later `claude auth logout` in this provider space sticks" — i.e.
AgentMux will not silently re-import a fresh global `~/.claude` login once
this shared dir has been explicitly logged out, by design, to avoid
clobbering an intentional sign-out.

The self-heal path that *can* recover a stale-but-present token
(`refresh_claude_dir_from_global_if_stale`, `cli_handlers.rs:770-807`, "the
Pozl 401" — a named recurring repro in the comments, not documented
elsewhere in the repo under that name) is only reached when a token exists
and fails validation. It is never reached when the token is empty, so an
empty-token shared dir is a dead end that **only** an actual completed
login (§1/§2's gap) or a manual file operation can clear — which is exactly
why a working `/login` matters here: absent the sentinel/self-heal gap
being separately revisited, `/login` succeeding is the only supported way
out of this state.

No scheduled process, cron job, or cleanup task in this repository writes
to this dir (checked: `agentmux-srv/src/backend/cron/*`,
`agentmux-srv/src/identity/cleanup.rs` — the latter is delete-triggered,
not scheduled, and is containment-guarded to
`~/.agentmux/shared/identities/`, which structurally excludes
`~/.agentmux/shared/providers/claude/`; see `cleanup.rs:140-146` and its
test `oauth_dir_outside_data_root_is_skipped`). The zeroing is consistent
with an explicit `claude auth logout` (or CLI-internal token-expiry
self-clear) having run against this dir at 07:43 — not with any AgentMux
background process. This is included for completeness; it is not this
retro's main finding and is not re-litigated further here.

## 4. Reinforcement — how this closes for good instead of narrowing again

1. **Make `/login` and "Login Again" try the visible-console path
   automatically on `no-url`, instead of erroring and pointing at a
   different button.** `forceProviderLogin` already knows the outcome was
   `"no-url"` (`login.ts:51`); on that outcome it should invoke
   `open_login_terminal` itself (fire-and-forget, exactly as the "Login via
   terminal" button already does) rather than surface an error asking a
   human to do it. Keep the piped attempt first — it's cheap, and if a
   future CLI version does print a scrapeable URL, the fast path still
   wins — but no-url should fall through automatically, not dead-end.
2. **Port `open_login_terminal` to macOS/Linux.** Today it's `Err("not yet
   implemented")` outside Windows (`platform.rs:1337-1342`), which means
   the *only* working escape hatch for this entire failure class doesn't
   exist on two of three platforms. `xterm -e`/`open -a Terminal.app` per
   the existing `// Tracked follow-up` comment.
3. **Cross-link the two retros that created this gap** — add a pointer from
   `retro-windows-terminal-window-leak-2026-06-21.md` to this doc (and vice
   versa) so a future change to pane-spawn stdio handling surfaces the auth
   consequence, and the reverse: a future auth-capture change checks
   whether the window-leak fix still applies.
4. **A standing test, not another spec note** — per the lesson already
   written twice in `docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`
   and `docs/retro/retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md`
   ("a written invariant is worthless if nothing re-checks it when the
   ground moves"): add a test asserting that for any provider whose login
   flow is known not to produce a scrapeable URL in a headless spawn (a
   flag/allowlist, keyed off the same signal `open_login_terminal`'s doc
   comment already encodes for Claude v2.1.x), the `no-url` outcome results
   in an automatic terminal-fallback attempt, not a terminal error state.
   This should fail loudly if a future refactor of `login.ts`/`force-login.ts`
   reintroduces the dead-end branch as the last step.
5. **Resolve the "Pozl 401" name.** It's cited twice in
   `cli_handlers.rs` (`:426`, `:776`) as a specific, apparently
   well-understood repro, but is undocumented anywhere else in the repo. If
   it refers to the "Poal"/"Nark" divergence case in
   `docs/reports/REPORT_AGENT_AUTH_DIVERGENCE_2026_06_20.md`, rename the
   comment to cite that doc directly; if it's a distinct case, write it up
   so the next person reading `cli_handlers.rs` doesn't have to guess.

## 5. Implemented same-day

Items 1, 2, and 4 of §4 shipped immediately after this retro was written,
without waiting for a second manual-recovery incident to justify them:

- **`runProviderLogin`** (`frontend/app/view/agent/flows/run-provider-login.ts`,
  new) — the shared three-tier orchestrator: `forceProviderLogin` (URL
  capture) → `seedGlobalLogin` (Claude-only, copy a valid global login) →
  `openLoginTerminal` + poll (real terminal, up to 5 min). `/login`
  (`commands/global/login.ts`) and "Login Again" (`useAgentControllerStatus.ts`'s
  `relogin`) both call it now instead of dead-ending on `forceProviderLogin`'s
  bare `"no-url"`. The manual "Login via terminal" button's own poll loop was
  deduped against the same helper (`pollForGlobalLoginSeed`,
  `flows/seed-global-login.ts`) rather than left duplicated.
- **`open_login_terminal` ported to macOS/Linux** (`agentmux-cef/src/commands/platform.rs`) —
  macOS writes a disposable `.command` script (env vars can't be passed to
  `open -a Terminal` directly) and opens it in Terminal.app; Linux tries
  `x-terminal-emulator` → `gnome-terminal` → `konsole` → `xterm` in order.
  Previously `Err("not yet implemented")` on both.
- **A standing test**, not just this doc: `run-provider-login.test.ts` pins
  the tier order, the Claude-only gating of tier 2, the config-dir-env
  stripping before tier 3, and that a cancelled/failed tier 3 doesn't hang
  the caller.

Item 3 (cross-linking the window-leak retro) is done inline above. Item 5
(the "Pozl 401" naming) is intentionally left open — resolving it needs
someone who was actually present for that repro to confirm the name, not a
guess dressed up as a citation.

## 6. What this retro is explicitly not

Not a claim that the piped/PTY capture path (`run_cli_login`/
`run_cli_login_pty`) is badly built — its URL-scraping, ANSI/OSC-8
handling, and pipe-draining fix (`platform.rs:532-539`, avoiding an earlier
EPIPE hang) are solid engineering for the CLIs where it works. Not a claim
that `CREATE_NO_WINDOW` piped spawning for agent panes was the wrong call —
it correctly fixed a real window-leak bug and should stay. The claim is
narrower: AgentMux already built the correct fallback for the one CLI where
the primary path is known to structurally fail, and then didn't call it
automatically from the two places (`/login`, "Login Again") a stuck user
actually reaches for.
