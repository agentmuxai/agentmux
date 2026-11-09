# SPEC: agents can point a Claude Code agent at another model server

**Author:** Agent2-5
**Date:** 2026-11-08
**Status:** implemented — #6874 (`crates/srv/src/backend/spawn_env/endpoint.rs`, `crates/srv/src/server/app_api/agent_env.rs`)

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
| `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY` | `vault:<entry id>`, or for a server that ignores the key one of a fixed list of words (`ollama`, `lm-studio`, `lmstudio`, `llamacpp`, `vllm`, `local`, `none`, `dummy`, `placeholder`, `unused`, `not-needed`, `no-key`); anything else is refused, because any shape that fits a placeholder also fits a short real key |
| `API_TIMEOUT_MS` | digits |
| `ANTHROPIC_MODEL`, `ANTHROPIC_SMALL_FAST_MODEL`, `ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL` | a model name, no spaces |

The endpoint names are written in capitals (a second spelling would be a second way to write the
same setting), and the launch reads them without regard to case all the same, so an environment saved
from My Agents in another case cannot send the sign-in to a custom server.

Other harnesses declare no base-URL variable today, so no other names are opened.
A `vault:` key must be an entry the person opened to the target agent **and** to the agent that writes
it (srv does not hand a key to a server for an agent that could not have read it); an agent on another
machine names none. srv reads it at launch, records the use in the Vault's audit, and the value never
appears in an answer, the Audit row, a log or the UI (a credential-named value is shown as `[hidden]`).
A refused key gets one answer, whether the entry is missing, of another kind or in a closed Vault, so
a refusal never says which entries exist; the reason is in srv's log.

Refused with a reason when a call sets one of these names: an agent on a model route (the route sets
server, key and models), and a container agent (its image is given the person's account; follow-up).
Removing the settings, or setting `ANTHROPIC_MODEL` alone, stays allowed for both.

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
- The first switch away from Anthropic starts a fresh conversation, because the config directory changes.
  Pointing an agent from one custom server to another keeps the directory, so its session is resumed on
  the new server (follow-up: end the conversation when the address changes).
- The login variables (`CLAUDE_CODE_OAUTH_TOKEN`, and the token or key the endpoint did not give) are
  removed from the child command itself: the child inherits srv's own environment, which may hold the
  person's token.
- srv's own background calls for the pane (title, summary, next-prompt suggestion) are built from the
  pane's saved environment with the shared sign-in directory, so for an agent on a custom server they
  make no call at all (`ambient/cli.rs`): sending the person's sign-in to that server is what the rule
  forbids, and sending the conversation to Anthropic is what pointing the agent elsewhere avoids. Its
  pane title keeps the last one it had.

## 4. Notice and undo

Every change that moves the address (`ANTHROPIC_BASE_URL` set, changed or taken away; a new key,
timeout or model on the same address is not one) opens a notice (`agent:endpoint-notice`): a card in each pane
of the target agent and a line in My Agents' Environment panel ("AgentY pointed Lolly at …"). For
30 seconds it carries an Undo (`undoagentendpoint`) that restores the previous text, only while the
environment is unchanged since. srv holds the previous text in memory; the event carries no key or
old text. Each change and each undo is an Audit row naming the acting agent.

## 5. Follow-ups

- Container agents: a broker that mounts a private config instead of the account.
- Equivalents for other harnesses, when one declares a base-URL variable.
- #30141/#30142 (a local-server route) build on this; a route still takes precedence over a
  hand-set endpoint.
