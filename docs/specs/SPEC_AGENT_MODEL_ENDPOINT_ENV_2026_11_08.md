# SPEC: agents can point a Claude Code agent at another model server

**Author:** Agent2-5
**Date:** 2026-11-08
**Status:** implemented

---

## 1. Problem

`Agents env_set` refused every `ANTHROPIC_*` name, so an agent could not point another Claude Code
agent at a model server of its own (a local server, a gateway, a hosted route). The person had to
edit the Environment in My Agents by hand. Two things made a plain allow unsafe:

- At launch srv puts the person's Claude sign-in in every Claude Code agent's environment. A
  custom `ANTHROPIC_BASE_URL` would receive that token.
- srv adds its own `--model` to the command line. For a custom server that is the wrong model name.

## 2. What an agent may set

On `Agents env_set` (and `lines`), for a Claude Code agent:

| Name | Value rule |
|---|---|
| `ANTHROPIC_BASE_URL` | `http(s)` URL with no user info, query or fragment |
| `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY` | `vault:<entry id>` or a short non-secret placeholder (for example `ollama`); a secret-shaped value is refused |
| `API_TIMEOUT_MS` | digits |
| `ANTHROPIC_MODEL`, `ANTHROPIC_SMALL_FAST_MODEL`, `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL` | a model name, no spaces |

Other harnesses declare no base-URL variable today, so no other names are opened.
A `vault:` key must be an entry the person opened to the target agent; srv reads it at launch,
records the use in the Vault's audit, and the value never appears in an answer, the Audit row, a log
or the UI (a credential-named value is shown as `[hidden]`).

Refused with a reason: an agent on a model route (the route sets server, key and models), and a
container agent (its image is given the person's account; follow-up).

## 3. Launch

`spawn_env/endpoint.rs` runs after the identity step and before the gateway env, so a model route's
own environment still wins:

- Base URL absent or `https://api.anthropic.com`: nothing changes.
- Otherwise the person's `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY`
  are removed, `CLAUDE_CONFIG_DIR` is a private directory per agent (so the shared sign-in file is not
  read), and the agent's own key (or the `agentmux-no-key` placeholder) is set. A step that fails
  strips the account variables rather than leave them in.
- `agent_runtime.rs` replaces srv's nominal `--model`/`--effort` with the endpoint's model
  (`ANTHROPIC_MODEL`, else the tier alias from `ANTHROPIC_DEFAULT_*_MODEL`).
- A resumed conversation starts fresh, because the config directory changed.

## 4. Notice and undo

Every change that touches these names opens a notice (`agent:endpoint-notice`): a card in each pane
of the target agent and a line in My Agents' Environment panel ("AgentY pointed Lolly at …"). For
30 seconds it carries an Undo (`undoagentendpoint`) that restores the previous text, only while the
environment is unchanged since. srv holds the previous text in memory; the event carries no key or
old text. Each change and each undo is an Audit row naming the acting agent.

## 5. Follow-ups

- Container agents: a broker that mounts a private config instead of the account.
- Equivalents for other harnesses, when one declares a base-URL variable.
- #30141/#30142 (a local-server route) build on this; a route still takes precedence over a
  hand-set endpoint.
