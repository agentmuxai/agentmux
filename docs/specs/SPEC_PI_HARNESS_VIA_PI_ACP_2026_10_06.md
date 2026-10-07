# Spec: Make the Pi harness launch, through the `pi-acp` adapter

**Status:** implemented (#4424)
**Date:** 2026-10-06
**Author:** Camper

## Ask

Pi agents should launch. Today every one fails at once.

## What happens today

`providers.rs` `PI` (mirrored in `catalog.ts`) runs Pi as an ACP agent:
`controller_type: Acp`, `launch_args: ["--json"]`, package
`@mariozechner/pi-coding-agent@0.73.1`.

Checked on 2026-10-06 against that pinned install:

- `pi --json` exits with `Error: Unknown option: --json`. Pi has
  `--mode text|json|rpc`, and none of them is ACP. Sending the ACP
  `initialize` request gets no answer, so no Pi launch has ever started.
  (`copilot --acp`, by contrast, answers `initialize` correctly.)
- Pi moved packages. `@mariozechner/pi-coding-agent` stops at 0.73.1; Pi is
  now `@earendil-works/pi-coding-agent`, at 1.0.4, needing Node 22.19+.

## Approach: the `pi-acp` adapter

`pi-acp` (npm, github.com/svkozak/pi-acp, 0.0.34, about 250,000 downloads a
week, the adapter Zed uses) speaks ACP over stdio and runs `pi --mode rpc`
behind it. AgentMux's ACP controller already drives Copilot and OpenClaw the
same way, so Pi needs no new protocol code.

Its constraints:

- It needs `pi` 0.81+ on PATH; it runs whatever `pi` it finds.
- It depends on `@agentclientprotocol/sdk`, `cross-spawn` and `zod`.
- It stores a session map in `~/.pi/pi-acp/session-map.json`, and Pi keeps
  sessions in `~/.pi/agent/sessions/`. Check how both behave under the
  per-agent `PI_HOME` isolation AgentMux sets.

The alternative, driving `pi --mode rpc` or `--mode json` directly, means a
new controller or translator for a protocol `pi-acp` already bridges. This
is not worth it unless the adapter proves unreliable.

## Change

1. **Two packages, one install.** `ProviderConfig::companion_npm_packages`
   lists packages installed with the pinned one; `npm_install_specs` gives
   every `name@version`. Both `install.start` and `cli_install` (the
   launch-time install and the startup warm-up) pass all of them to one
   `npm install --prefix …`, under the same lock and completion marker. Pi:
   `pi-acp@0.0.34` plus `@earendil-works/pi-coding-agent@1.0.4`.
2. **Launch** `pi-acp` (`cli_command: "pi-acp"`, no launch args). pi-acp runs
   whatever `pi` is on PATH unless `PI_ACP_PI_COMMAND` names one, and the
   managed install's `.bin` is not on PATH, so the ACP controller sets
   `PI_ACP_PI_COMMAND` to the `pi` beside `pi-acp` (`pi_beside_pi_acp`,
   blockcontroller/acp.rs), and the login terminal gets the same variable
   (`companionCliEnv`).
3. **Sign-in.** pi keeps its logins and API keys in `PI_CODING_AGENT_DIR`
   (default `~/.pi/agent`). AgentMux had set `PI_HOME`, which pi never reads;
   `authConfigDirEnvVar` is now `PI_CODING_AGENT_DIR`, so the agent, the
   sign-in check and the login terminal all use AgentMux's shared Pi dir, one
   for every Pi agent, as `cli-managed` (no account to bind). pi-acp has no
   status command; for a `pi-acp` path srv runs the `pi` beside it with
   `--list-models` (`run_auth_check`), which exits 0 either way and says "No
   models available" until a provider is set up. An empty check command is
   now refused by srv, so no caller can read a bare CLI's exit 0 as a
   sign-in. An unauthenticated session also fails `session/new` with
   "Authentication required". Login is pi-acp's own terminal method,
   `pi-acp --terminal-login`.
4. **Pins and prereqs:** Node 22.19.0 (pi's `engines`). The old-name pin is
   gone.
5. **Card:** the description was already corrected (#4403).
6. **Startup instructions go to `AGENTS.md`.** pi 1.0.4 treats
   `.pi/APPEND_SYSTEM.md` (the earlier target) and `.pi/SYSTEM.md` as
   trust-protected; in RPC mode, which pi-acp uses, a project with no saved
   trust decision skips them, so AgentMux's instructions were never read.
   `AGENTS.md` is a context file pi always loads, so no trust decision is
   needed (trusting every project would also load any repo's own `.pi`
   settings and extensions unasked).

## Tests

- `providers.rs`: Pi runs `pi-acp` with no launch args, and installs
  `pi-acp@0.0.34` plus `@earendil-works/pi-coding-agent@1.0.4`; every other
  provider installs one package.
- `pi_beside_pi_acp` finds the `pi` beside `pi-acp`, and only for pi-acp;
  `pi_lists_models` reads "No models available" as signed out; an empty check
  command is refused.
- Frontend: `companionCliEnv` for the login terminal; Pi is `cli-managed`,
  checked with `--list-models`, with `PI_CODING_AGENT_DIR` and Node 22.19.0.
- Manually, before the change: `pi-acp` over ACP answers `initialize`, and
  `session/new` without credentials fails with "Authentication required"
  and offers the terminal login.
- Manual, in a `task dev` build: install Pi from the new-agent screen, create
  an agent, and see the authentication message. A full turn needs a Pi
  provider login, which this machine doesn't have.

## Open questions

1. ~~Does the session map follow `PI_HOME`?~~ pi reads `PI_CODING_AGENT_DIR`,
   which AgentMux now sets (see 3).
2. Which accounts and model providers does Pi need configured to answer a
   first prompt (Pi brings its own model providers)?
