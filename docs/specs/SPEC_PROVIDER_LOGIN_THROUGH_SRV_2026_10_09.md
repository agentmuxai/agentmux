# SPEC: One provider-login implementation, in srv

**Date:** 2026-10-09
**Status:** proposed
**Author:** Agent4
**Builds on:** `PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md` (one login orchestration in the frontend), `SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` (srv's `auth.*`), `SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` (the host's capture), `SPEC_SRV_HEADLESS_MODE_2026_09_26.md`.

## 1. Problem

Two independent implementations log a provider CLI in:

| | Host path | srv path |
|---|---|---|
| Runs the CLI | the CEF host: `run_cli_login`, `run_cli_login_pty` (`crates/cef/src/commands/cli_login.rs`) | srv: `auth.start` (`server/identity_handlers.rs`, `identity_auth_spawn.rs`) |
| Used by | `runProviderLogin` (`/login`, Login Again, Claude and OpenClaw in the New Agent modal, the Claude login panel) | `AuthFlowController` (other providers in the New Agent modal) |
| Paste a code | `set_provider_auth`, one global slot | `auth.submitcallback`, per session |
| Terminal fallback | `open_login_terminal`: a native console window | none |

The host path learned things the srv path didn't (below), and the srv path is per-session and saves the account itself, which the host path leaves to frontend code. A UI attached to a headless srv has no host, so it can only use the srv path, which can't yet log Claude in.

## 2. Goal

**srv is the one place a provider CLI is logged in, for every host.** The host's part shrinks to opening a URL (`AppApi.openExternal`). The terminal fallback becomes an ordinary srv terminal pane running the CLI's login command. The host's login commands are then deleted.

## 3. What the srv path needs first

From a side-by-side audit, highest risk first:

1. **Claude's login under a PTY.** The host needed three things srv's PTY spawn lacks:
   - answering the cursor-position query (`ESC[6n`) Claude blocks on (issue #2429);
   - a 4096-column PTY, so the URL isn't hard-wrapped and truncated;
   - resolving the npm `.cmd` shim on Windows (`cmd.exe /c` hangs under ConPTY).
2. **URL capture that survives terminal codes.** srv matches raw lines, so colour codes or an OSC-8 hyperlink defeat it, and its Claude pattern only knows the old `console.anthropic.com` URL.
3. **No leaked login processes.** A timed-out srv session is marked failed only when polled and its child is never killed; nothing reaps a session whose client went away, and the session map is never pruned. The host has a 6-minute reaper.
4. **One login at a time per account directory.** srv sessions can run concurrently against the same directory.
5. **A terminal fallback**, for CLIs whose URL capture fails: a srv terminal pane running the login command with the account's environment, and a server-side check that it succeeded.
6. **The steps after success** that live in frontend code on the host path: linking the agent, refreshing a live pane's environment, checking the credentials actually changed.

## 4. Phases

| Phase | Content |
|---|---|
| **L1** | §3.1–3.2. The host's DSR responder, PTY width and URL extraction move to `agentmux_common::login_pty` (one copy, with their tests); srv's PTY spawn uses them and resolves the shim; srv's line matching runs on text with terminal codes removed, preferring OSC-8 link targets, and knows Claude's current URL |
| **L2** | §3.3–3.4. srv kills a session's child on timeout and when it ends, reaps sessions nobody polls, prunes finished ones, and allows one live login per account directory |
| **L3** | §3.5. The terminal fallback as a srv pane |
| **L4** | §3.6, and the frontend: `runProviderLogin` and `AuthFlowController` drive srv's `auth.*` for every provider; the host only opens the URL |
| **L5** | Delete the host's login commands (`run_cli_login`, `run_cli_login_pty`, `cancel_cli_login`, `get_cli_login_status`, `open_login_terminal`, `set_provider_auth`) and the frontend code that only served them |

Each phase is useful alone: L1 fixes Claude in the New Agent modal's srv path, L2 fixes leaks that exist today.

## 5. Known limit

A CLI that finishes login by redirecting the browser to `localhost` on the machine it runs on (Codex on port 1455, Gemini) only completes when the browser can reach that machine. Claude's paste-the-code flow and device-code flows (Copilot, Codex's `--device-auth`) work from anywhere. The terminal fallback (L3) is the way out for the rest.

## 6. Testing

Every phase keeps the existing unit tests passing where code moves, and moves them with it. L1 adds tests for srv's matching of a coloured line, an OSC-8 link and Claude's current URL, and for the shared reader answering a split query.
