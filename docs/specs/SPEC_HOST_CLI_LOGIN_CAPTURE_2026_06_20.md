# SPEC: Host-side CLI login capture is broken for Claude Code v2.1.183
**Date:** 2026-06-20
**Author:** AgentA
**Status:** Draft — revised 2026-06-20; **2026-06-23 capture outcome: §5.1 (`setup-token`) is a confirmed DEAD END under our spawn; §5.5 (seed-from-global) is now the PRIMARY path.**
**2026-08-03 update:** §0's "abandoned for Claude v2.1.x" verdict for §5.2 (paste-code) and §5.6
(URL-scrape) was correct for the CLI build tested here (v2.1.183) but does **not** hold for
v2.1.198+: live probes that day showed the pinned CLI now prints the full PKCE authorize URL under
our PTY spawn and accepts a pasted code on stdin, auto-completing on browser authorize with no
paste needed in the happy path. §5.5 (seed-from-global) remains a fast secondary path (no browser
round-trip when a valid global login exists), but §5.2/§5.6 are revived as the primary flow. See
`docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md` for the current design; §0 below is kept
verbatim as the historical record of what v2.1.183 actually did and why §5.5 was the only option
at the time.
**Related:** SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20 (frontend force-login, merged #1604); retro `docs/retro/retro-claude-v2-1-auth-spawn-2026-06-23.md`

---

## 0. CAPTURE OUTCOME + DECISION (2026-06-23) — READ FIRST

Two live captures (agents "Marks", "Lazo") on a `setup-token` build proved §5.1 **cannot work
in-app**: `run_cli_login: spawned (PTY)` → `no auth URL captured within 15s`, **zero `[login-pty]`
lines** (the v2.1.x login is a full-screen TUI that redraws with no newlines), and **no browser
opened**. Web research confirms why: the OAuth flow opens the OS default browser via a localhost
callback, but **in any spawned / piped / tmux / headless context it hangs silently** — and
`setup-token` shares that same browser front-end (the token only prints *after* an OAuth that
can't start under our spawn). Upstream's own remedy (issue #7100) is exactly seed-from-global:
authenticate once where a browser works, then seed the creds into the headless env.

**DECISION:**
- §5.1 (`setup-token` capture), §5.2 (paste-code), and §5.6 (URL-scrape) are **abandoned for
  Claude v2.1.x** — all founder on the un-spawnable TUI / no-browser problem.
- **§5.5 (seed-from-global) is the PRIMARY auth path.** Make it a first-class, up-front flow
  ("Use my existing login" offered on launch/create when a valid global login exists), not just a
  401-row fallback. `auth login`/`setup-token` survive only as the "no global login yet → go
  authenticate in a real terminal, then seed" instruction.
- **Salvage kept:** `redact_secrets()` + token-line detection in `run_cli_login_pty` (security —
  `setup-token` would otherwise log a live token). Also: a pending login PTY's 6-min reap can
  block host clean-exit (auth × lifecycle orphan leak — see lifecycle consolidation notes); the
  robust machine should cancel the reap on quit.

Evidence: [authentication](https://code.claude.com/docs/en/authentication) ·
[headless](https://code.claude.com/docs/en/headless) ·
[#7100](https://github.com/anthropics/claude-code/issues/7100).

---

## 1. Symptom

After the frontend force-login fix (#1604), clicking **Login Again** correctly
calls `run_cli_login` (confirmed in logs). But **no browser opens and no OAuth URL
box appears**. The agent stays un-authenticated.

## 2. Root cause

`run_cli_login` / `run_cli_login_pty` (`agentmux-cef/src/commands/platform.rs`)
spawns the provider login (`claude auth login`) in a PTY and **scrapes each output
line for an OAuth URL** via `extract_url`, with a 15 s cap. For Claude Code CLI
**v2.1.183** this never fires:

```
run_cli_login: spawned (PTY), waiting for OAuth URL  cli=…claude.cmd  pid=…
run_cli_login_pty: no auth URL captured within 15s          ← every time
```

Evidence gathered 2026-06-20 (repro) + 2026-06-20 (upstream-docs research):
- `claude auth login --help` exposes no URL-printing / headless flag
  (`--claudeai | --console | --email | --sso` only).
- Run in a **non-TTY pipe**, `claude auth login` prints **nothing** to stdout
  (exits on timeout) — it's a TTY-interactive TUI.
- In a **real terminal** it opens its own browser and runs a **localhost OAuth
  callback server** — it does NOT emit a plain `https://…` URL line for the host
  to scrape. Per upstream docs, the URL only materialises when the user **presses
  `c` to copy the login URL to the clipboard** — so there is *no printable URL
  line for `extract_url` to match* on this CLI version. This is the real reason
  the scrape model is obsolete, not an ANSI-stripping gap: `extract_url` already
  strips CSI/OSC-8 (`platform.rs:805`).

Two distinct failures stack here:
- **(A) URL not captured** → host can't open a browser (this spec). Confirmed
  cause: the URL is clipboard-on-`c`, never a stdout line.
- **(B) localhost callback unreachable** when the user logged in manually — made
  worse by *competing* login processes (host PTY attempt + diagnostic probes)
  leaving stale tabs whose callback servers had died. A clean single attempt is
  the precondition for any callback to land. This is the **inherent weakness of
  the RFC 8252 loopback-redirect flow** in sandboxed/non-default-browser contexts
  — see §3.5.

Orthogonal, already understood (SPEC_REAUTH §11): `claude auth status` reports
`loggedIn:true` for an **expired** token (checks presence, not validity) — so it
can't be used to gate OR to confirm a login. **New hypothesis to rule out in §4:**
upstream docs state that a stray `ANTHROPIC_API_KEY` in the environment *takes
precedence over* subscription OAuth (auth precedence #3 > #6) and, if the key
belongs to a disabled/expired org, produces exactly this "looks logged in but
401s" symptom. The agent env may be leaking one in.

## 3. Why the old model worked before

Earlier Claude CLI builds printed the OAuth URL as a plain line; `extract_url`
caught it, the host opened the browser (or, post-#1594, an in-app pane) and polled
`auth status` for completion. v2.1.183 moved to a self-driving TUI (opens its own
browser, runs its own callback, copies the URL to the clipboard on `c`), breaking
both the capture and the host-opens-the-browser assumption.

## 3.5 Research findings (upstream docs, 2026-06-20)

Anthropic now documents **three officially-supported auth paths that do not
require scraping a URL** ([code.claude.com/docs/en/authentication](https://code.claude.com/docs/en/authentication.md)).
This reframes the whole fix: stop fighting the transport, adopt a supported path.

1. **`claude setup-token` → `CLAUDE_CODE_OAUTH_TOKEN`** — the documented headless
   path. `claude setup-token` runs OAuth once and **prints a one-year token to
   stdout** (it saves nothing). Capture stdout, write the token into the agent's
   isolated env as `CLAUDE_CODE_OAUTH_TOKEN`. Precedence #5 — above subscription
   `/login` (#6), so it bypasses the un-driveable `auth login` TUI entirely.
   Caveats: requires Pro/Max/Team/Enterprise; **inference-only** (no Remote
   Control); **`--bare` ignores it**.
2. **Paste-the-code fallback (v2.1+)** — when the browser callback can't reach
   localhost (the docs explicitly name **WSL2, SSH, containers** — our PTY case),
   the CLI shows a login code in the browser and prompts
   **`Paste code here if prompted`** on stdin. This is the designed-for-sandbox
   path and the robust answer to failure (B). **Our infra already supports it:**
   `set_provider_auth` writes a code into `cli_login_stdin` over both pipe and PTY
   (`CliLoginStdin::write_line`, `platform.rs:373`). The missing piece is getting
   the user *to* the auth URL (open it host-side, or send `c` and read the
   clipboard) and surfacing the paste box.
3. **Env-var / credential precedence** — order is Bedrock/Vertex/Foundry →
   `ANTHROPIC_AUTH_TOKEN` → `ANTHROPIC_API_KEY` → `apiKeyHelper` →
   `CLAUDE_CODE_OAUTH_TOKEN` → subscription `/login`. Credentials live in
   `~/.claude/.credentials.json` (mode 0600; under `CLAUDE_CONFIG_DIR` when set;
   macOS uses Keychain). This **confirms §5.5 (seed-from-global)** as the
   industry-standard recovery — upstream issue #7100's recommended remote-auth
   method is literally "copy the credential file."

Standards context: Claude's flow (loopback redirect + PKCE) is exactly what
[RFC 8252](https://www.rfc-editor.org/rfc/rfc8252.html) mandates for native apps;
the loopback's known failure mode is "callback can't reach localhost," and the
standard mitigation is the manual code-paste fallback — i.e. path 2, not forcing
a subprocess browser-open (old §5.3).

## 4. Required first step — INSTRUMENT (don't design blind)

`run_cli_login_pty` only logs PTY lines **after** a URL is seen (target
`login_pty`, `platform.rs:713`); the pre-URL scan silently consumes everything.
We never see what `claude auth login` actually prints **before** the
(never-captured) URL. Before committing to a fix, log **every** PTY line from the
first byte, plus the child's exit code (already logged) and the auth-related env
keys present. Capture one real run. That tells us definitively whether claude:
  (a) prints a URL we're failing to match (→ unlikely given §2, but cheap to rule out),
  (b) opens its own browser + shows a paste prompt (→ path 2 / §5.1), or
  (c) silently does nothing in the host's PTY env (→ path 1 `setup-token` / §5.3).

Also assert in this step: **is `ANTHROPIC_API_KEY` present in the agent env?**
(§2 new hypothesis). Log the *names* of auth-related env keys (never values).

Implemented in this spec's first code slice (see §8).

## 5. Fix options (ranked, revised after §3.5)

### 5.1 `setup-token` capture → `CLAUDE_CODE_OAUTH_TOKEN` (NEW — preferred)
Change Claude's `login_args` from `["auth", "login"]` to `["setup-token"]`. Run it
in the PTY (still needs a TTY + browser for the one-time OAuth), but instead of
scraping a URL, **capture the token line it prints to stdout**, then write it into
the agent's isolated provider env / a place the agent's spawn env reads as
`CLAUDE_CODE_OAUTH_TOKEN`. Robust: it's the documented headless contract, and
completion is the token's *arrival*, not a fragile status poll. Pairs with §5.2
for the browser-open and §5.5 as the no-OAuth fallback.

### 5.2 Drive the paste-code flow (NEW — interactive fallback)
Stop scraping. Host opens the auth URL itself (obtain it by sending `c` to the PTY
and reading the clipboard, or — simpler — let claude open its own browser), the
user authorises, the browser shows a code, the user pastes it into the existing
auth box, and `set_provider_auth` forwards it to the child over the *already-wired*
`CliLoginStdin`. Detect completion by the credential's `expiresAt` advancing (NOT
`loggedIn:true`, which lies). This is the RFC 8252-sanctioned fallback and reuses
infra we already ship.

### 5.3 Status-transition poll (completion detector, not a driver)
Snapshot the credential's `expiresAt` (or its absence), then poll
`auth status --json` until a **new** credential appears — detect login by
`expiresAt` advancing. Keep as the *completion signal* layered onto 5.1/5.2; it
does not by itself make a browser open, so it is not a standalone fix (the
2026-06-20 repro showed no browser opened from the host's spawn).

### 5.4 logout-then-login (needed regardless)
A stale/expired-but-present credential makes `claude auth login` / `setup-token`
prompt/skip instead of starting a clean OAuth. For an explicit re-login, run
`auth logout` first (host-side, in the isolated `CLAUDE_CONFIG_DIR`) so the OAuth
always starts clean. Also `unset ANTHROPIC_API_KEY` for the child if §4 shows one
leaking (§2). Pairs with whichever capture fix lands.

### 5.5 "Use my existing login" — seed from global (ship now, low risk)
The reliable 2026-06-20 recovery was copying `~/.claude/.credentials.json` into the
agent's isolated dir (`~/.agentmux/shared/providers/claude/.credentials.json`).
Add an explicit button/flow: when a global Claude login exists and is valid
(`expiresAt` in the future), offer "Use my existing Claude login" which copies the
credential (incl. `refreshToken`, so it keeps refreshing). This is the documented
"seed isolated creds from global" pattern (cf. PR #1283) and the upstream-#7100
recommended remote-auth method — unblocks users even if the OAuth capture is never
fixed. Caveat: global and isolated then share a `refreshToken`; if Anthropic
rotates refresh tokens, one side can stale the other — acceptable as a recovery
path, document it.

### 5.6 `extract_url` fix — DROPPED
The old §5.2. `extract_url` already strips CSI/OSC-8 (`platform.rs:805`); the
problem is not an unmatched URL but **no URL line at all** (clipboard-on-`c`).
Keep `extract_url` for the providers that *do* print a URL (Codex/Gemini/OpenClaw),
but it is a dead end for Claude v2.1.x.

## 6. Recommendation

1. **Now:** ship **5.5** (seed-from-global button) as the reliable recovery — it
   needs no URL capture and works today.
2. **Now (this PR):** land the **§4 instrumentation** so a real run can be
   captured (small, safe, permanent-quality logging).
3. **Next:** capture one real `claude auth login` / `setup-token` run on a host
   build; confirm (a)/(b)/(c) and whether `ANTHROPIC_API_KEY` is leaking.
4. **Then:** implement **5.1 (`setup-token`)** as the primary capture — fall back
   to **5.2 (paste-code)** for the interactive case; layer **5.3** as the
   completion detector and fold in **5.4** for clean re-login.
5. Old §5.3 (force a subprocess browser-open) and old §5.2 (`extract_url` rework)
   are **dropped** for Claude — superseded by the supported paths above.

## 7. Test plan

- Reproduce by invalidating the isolated credential (expire `expiresAt` + corrupt
  `refreshToken`) so the agent 401s with `auth status` still `loggedIn:true`.
- Assert: failure banner + inline 401 node appear (regression guard on the merged
  visibility work).
- Assert: §4 instrumentation logs every PTY line from byte 0 + the auth-env key
  names for one real run.
- Assert (5.1): a completed `setup-token` run yields a token, and an agent spawned
  with `CLAUDE_CODE_OAUTH_TOKEN` set authenticates on its next turn.
- Assert (5.2): the paste box appears, a pasted code reaches the child, and a
  completed login writes a credential with an advanced `expiresAt`.
- Assert (5.5): seeds a valid credential when a global login exists.

## 8. Implementation log

- **2026-06-20 — slice 1 (§4 instrumentation):** `run_cli_login_pty` now logs
  every PTY line from the first byte (not just after a URL), and both spawn paths
  log the *names* of auth-related env keys present in `auth_env`
  (`ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` / `CLAUDE_CODE_OAUTH_TOKEN`) so the
  precedence hypothesis (§2) is checkable from logs. Names only — never values.
  Target `login_pty`, visible via `muxlog host grep login-pty`.
  Verified: `cargo check -p agentmux-cef` green (1m50s) after clearing a
  partial-CEF-extraction build blocker (see memory `CEF extraction race vs
  Windows file scanners`). Not yet smoke-tested against a live login run (needs a
  host build + interactive OAuth — §4's capture step).

- **2026-06-20 — slice 2 (§5.5 ship-now recovery): "Use my existing Claude login".**
  New host IPC command `seed_provider_auth_from_global`
  (`agentmux-cef/src/commands/providers.rs` + `ipc.rs`): reads the user's GLOBAL
  Claude credential (`$CLAUDE_CONFIG_DIR` if the host inherits one, else
  `~/.claude/.credentials.json`), validates `claudeAiOauth.expiresAt` is in the
  future (mirrors `agentmux-srv` `identity::resolver::probe_oauth_status`), and
  copies it verbatim — incl. `refreshToken`, so the isolated session keeps
  refreshing — into the agent's isolated dir
  (`~/.agentmux/shared/providers/claude/.credentials.json`) via temp+rename.
  Returns `{ seeded, status: seeded|missing|expired, expiresAt }` (no token
  material). Frontend: `seedProviderAuthFromGlobal` API
  (`cef-api.ts`/`custom.d.ts`), a `seedGlobalLogin` flow, `useGlobalLogin` on
  `useAgentControllerStatus`, and a "Use existing login" 🌐 action beside
  "Login Again" on the 401/403 failure CTA (`failure-accessory.ts` +
  `useAgentFailure` + `agent-view.tsx`). No restart needed — the running agent
  re-reads its credential per request. Verified: `cargo check -p agentmux-cef`
  green; `vitest` 18/18 on the two affected suites (incl. a new CTA assertion);
  `tsc --noEmit` clean for every touched file (31 pre-existing errors are all in
  unrelated files). NOT yet wired into the PRE-LAUNCH auth panel (the srv
  `auth.*` state-machine path) — tracked as a follow-up. Live smoke (real
  expired isolated cred + valid global) still pending.
