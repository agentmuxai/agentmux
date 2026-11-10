# Spec: an agent is a harness, a model route and a key: spawn on any model and switch the model in place

**Date:** 2026-11-06 (task #27458; the build follows it)
**Status:** active — P1 (catalog and gateway) in PR #6662; P2 (spawn, create, update and routes) in PR #6673; P3a (the switch in srv, the panel's two commands and the real model on the pane) in PR #6825; P3b (the panel itself) in PR #6849; P3c's image count in srv in the PR after it (the first-turn commit and "Compact, then switch" remain); P4 and P5 not started. Approved by the operator 2026-11-06 ("greenlight 27458"; the recommendations in section 12 are the decisions). Verified 2026-11-06.
**Author:** Agent5@narko, for AgentX's lane
**Read from:** `origin/main` at `1a79691b0` (v0.60.18). Web facts read the same day are marked.
**Builds on:** the AgentSky parity and model switching report of 2026-11-05 (task #24714, kept outside the repo: one agent, one id, one history), PR #6555 (`Agents` create/update/bundles).
**Related:** `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md`, `SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md`, `SPEC_CONTAINER_AGENT_AWS_CREDENTIALS_2026_10_31.md`

## 1. The ask, and the answer in one page

The operator, 2026-11-06: "would you be able to spawn a new full agent with a claude code using the latest deepseek model? we also want to support such requests from users, so it needs to part of muxapi and available to agents. we also want to start defaulting to container agents. you'd use the openrouter key we have". Then, through AgentY: "the custom setup is actually related to the earlier agentsky parity work. essentially, we want more harness/model flexibility".

So this is not an OpenRouter feature. It is one model that three things use:

> **An agent runs a *harness* (Claude Code, Codex, ...) on a *model route* (Anthropic, OpenRouter, DeepSeek direct, ...) with a *key* the route names by reference.** Spawn and create choose all three. **The harness is fixed for the agent's whole life.** It is chosen once, at create or spawn (the `harness` parameter), and is read-only after that; nothing in this spec or its build changes an existing agent's harness. What can change on a running agent is the route and the model, through the composer's existing model/effort panel, which relaunches the same session. Switching the harness is not part of this spec (section 8).

What I recommend, in five lines:

1. **A route is a named entry AgentMux owns** (base URL, protocol, key reference, models, prices). An agent or a person picks a route by name. Nobody types a URL or a key into a definition.
2. **The key stays in srv.** The agent's harness is pointed at a gateway in srv, with a per-agent token (the pattern container agents already use), and srv adds the real key upstream. The key is never in the agent's environment, its container, its transcript or its Bash tool's reach. The same gateway counts spend per agent.
3. **Spawn and create take `harness`, `route` and `model`.** The New agent form takes the same three with the same validation. `Agents` gains `routes` to list what can be chosen.
4. **New agents default to a container** when Docker is there, and `native` stays one explicit word away. Existing agents keep what they have.
5. **The model is switched in the composer's model/effort panel**, which grows from the Claude models to every model on the routes that serve the agent's harness. The agent card and the lists only display the current route and model.

## 2. What exists today (read from `origin/main`)

| Piece | Where | What it does |
|---|---|---|
| A base URL per agent | `AgentDefinition.model_vendor_base_url`; `resolve_vendor_env_override` (`server/app_api/agent_open.rs`) | Sets the provider's `base_url_env_var` (Claude: `ANTHROPIC_BASE_URL`) at launch. A set value makes the agent's vendor `"custom"` (`Store::resolve_effective_vendor`). Only Claude declares a variable (`base_url_env_var: Some(...)`); no other provider does, so the field cannot redirect any other harness. |
| No route, no key, no model id | `ProviderConfig.supported_vendors` (`claude`: `["anthropic"]`, `qwen`: `["openrouter"]`) | A vendor is a label for the provider's one default; a harness has one vendor. Nothing names a route or a key. |
| Keys as accounts | `IdentityAccount` (`provider`: github, aws, anthropic, custom; `kind`: pat, role, api_key, env_ref) with `SecretRef::Keychain`; linked to an agent with `agent_identity_link`; `identity/resolver/inject.rs` puts an `anthropic` account's key in `ANTHROPIC_API_KEY` at spawn | A key in the OS keychain, injected into the harness's environment. For `custom` nothing is injected (an existing test pins it). |
| Keys in the Vault | The Vault (`api-key` entries); an agent uses an entry by name in one command's environment (`Terminal run` with `secrets`), after a per-agent, per-entry approval, with output scrubbed | Built for a command, not for an agent's own process. |
| Spawn | `POST /api/v1/agent/spawn` (`app_api/agent_spawn.rs`) copies the **leader's** definition, so the worker has the leader's harness. `worker_runtime` accepts a model only for Claude and only `opus`, `sonnet`, `haiku` or a `claude-…` id ("unknown model … use opus, sonnet, haiku or a claude-… model id"). The person approves each spawn (`consent`), and the question names the model. | A worker cannot be on another harness or another model vendor. |
| Create | `POST /api/v1/agent/create`: a named agent from a template; `account`, `runtime`, `container_image` | The template decides the harness. |
| Runtime default | `runtime` is "as you run" for spawn and "the template's" for create; `AgentDefinition.agent_type` defaults to `"host"` ("so a missing field never silently starts the wrong runtime") | Native unless asked. |
| The operator-only guard | `Agents update` refuses `model_vendor_base_url` (and `shell`, `provider_flags`, `working_directory`): they "decide … where its traffic goes" (PR #6555) | Right for a free URL; wrong for a catalog route (section 5.4). |
| Container credentials | `backend/container/credential.rs`: a scoped `amxc_…` token a container agent carries instead of the instance key, accepted only on `container_route_allowed` routes and attributed to the agent | The pattern the gateway reuses. |
| Remembered runtime | `agentlastruntime` (`agent_runtime::Remembered`): the model and effort an agent last ran with | Where an in-place model switch is recorded today. |
| API reachability | `connectivity` tracks `model_api:<vendor>` and `model_api:<vendor>@<host>` for a custom address | The status bar already shows a custom endpoint's health. |
| OpenRouter | In the frontend catalog and `qwen.rs` only (`supported_vendors: ["openrouter"]`; Qwen Code takes an OpenAI-compatible endpoint through its own `OPENAI_BASE_URL`, which srv does not set) | Not wired for Claude Code. |

## 3. The model

### 3.1 A route

A route is an entry in an srv-owned catalog. Built-ins ship with AgentMux; the operator may add more (decision D5).

| Field | Meaning |
|---|---|
| `id` | `anthropic`, `openrouter`, `deepseek`, ... |
| `harnesses` | which harnesses it serves, each with its protocol: `claude` speaks the Anthropic Messages protocol; `codex` and `qwen` speak OpenAI's |
| `base_url` | fixed by AgentMux for a built-in. Never read from an agent's request. |
| `key` | a reference to a Vault entry (by id; its label is shown), or `login` for Anthropic's own sign-in, which stays as it is |
| `models` | the ids it serves, with a plain label, context size, capability flags (tools, thinking, images, caching) and a price per million tokens in and out |
| `roles` | which model fills the harness's roles: main, small and fast, subagent. Claude Code has three tiers and a subagent model; a route says what each maps to. |

Built-in routes to start: **`anthropic`** (today's behaviour, unchanged), **`openrouter`** (`https://openrouter.ai/api`, Anthropic protocol for Claude Code), **`deepseek`** (DeepSeek's own Anthropic-format address, `https://api.deepseek.com/anthropic`, from a web search result I did not open on DeepSeek's site; check before shipping). Bedrock and Vertex later.

OpenRouter's current DeepSeek models, read from its public API today: `deepseek/deepseek-v4-pro-0813` (1,048,576 context, tools supported, 27 endpoints) and `deepseek/deepseek-v4.1-flash-1017` (1,048,576 context, tools supported, 59 endpoints). The model list is read from the route's own models endpoint and cached, as the Anthropic catalog already is (`backend/model_catalog.rs`), so "the latest DeepSeek" is a lookup and not a constant in our code.

### 3.2 What the definition keeps

`AgentDefinition` keeps `provider` (the harness; read-only after creation, as today) and gains `model_route` (a route id; empty means the harness's default) and `model` (an id the route serves; empty means the route's default). `model_vendor_base_url` stays as the operator's way to a custom address and is shown as the route `custom`. The agent keeps one id, one history and one Personal Memory whatever its route (report #24714, decision 1).

### 3.3 What Claude Code on OpenRouter needs

From OpenRouter's Claude Code guide, read today: `ANTHROPIC_BASE_URL="https://openrouter.ai/api"` (no trailing slash), `ANTHROPIC_AUTH_TOKEN` set to the OpenRouter key, and `ANTHROPIC_API_KEY=""` explicitly empty. Model choice is by `ANTHROPIC_DEFAULT_OPUS_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`, `ANTHROPIC_DEFAULT_HAIKU_MODEL` and `CLAUDE_CODE_SUBAGENT_MODEL`; the `[1m]` suffix on a slug marks a 1M-context model so Claude Code does not compact early. The guide warns that Claude Code "is optimized for Anthropic models and may not work correctly with other providers".

With the gateway (section 4) the launch environment becomes: `ANTHROPIC_BASE_URL` = srv's gateway address for this route, `ANTHROPIC_AUTH_TOKEN` = the agent's gateway token, `ANTHROPIC_API_KEY=""`, the three tier variables and the subagent variable from the route's `roles`. These names are reserved already (`agent_env::provider_names`), so an agent cannot set them itself.

## 4. Keys: why a gateway

**The problem.** A key in the harness's environment is in the environment of everything the harness starts: Claude Code's Bash tool can run `env`. "Never visible to the agent" is not true of an environment variable, for a host agent or a container one. The Vault's own spec says its scrubber "is a guard, not a boundary", for a command the person approved once; here the process is the agent itself, always on.

**The design.** srv hosts a model gateway: `POST /api/v1/model/<route>/...`, a streaming pass-through. The harness is pointed at it with a per-agent token (minted like the container token: scoped, attributed to the agent, valid only for this route's gateway paths and `container_route_allowed`). srv looks up the Vault entry at request time, adds the upstream authorization, forwards, and streams the answer back. So:

- the key is never in an environment, a container, a transcript, a log or the process list; it is in srv's memory for the length of a request;
- it works for a container agent without a credential in the container (the operator's rule: credentials are brokered per use);
- the same path counts tokens and cost per agent (section 9) and can stop an agent that reached its cap without killing the process;
- the model id can be rewritten (a harness that insists on a Claude name is mapped to the route's model), and a route's quirks can be fixed in one place;
- a route can be switched or revoked without touching the agent's environment.

**What it costs.** One streaming proxy in srv (the stack has `axum`, `reqwest` and the container-token middleware). Its cost is a hop on localhost and a new place that must not buffer a whole stream. This is the largest piece of the build (section 13, P1).

**The simple alternative (not recommended).** Inject the key as `ANTHROPIC_AUTH_TOKEN` at launch from an `IdentityAccount`, as `ANTHROPIC_API_KEY` is today. It ships sooner and it exposes the key to the agent and its subprocesses. If chosen (D3), the entry must be an OpenRouter key made for this and capped at OpenRouter, and the Agent card must say the agent can read it.

**The Vault.** The route's key is a Vault `api-key` entry whose label starts with the route's key label ("OpenRouter API key" for OpenRouter). Resolved at each request, shown by label. Three cases:

- *The entry is missing or locked:* the first request answers the pane with a Vault request card (`Vault request`, the existing card spec), the agent's turn waits as it does for any ask, and the person answers once.
- *The upstream says the credit is out or the key is refused (402, 401):* the gateway marks the route unusable for that agent, the pane shows one card ("OpenRouter says the credit is spent"), and the agent is not retried in a loop. `agents/failure.rs` already classifies Anthropic's errors; this adds the route's.
- *The key is rotated in the Vault:* the next request uses it; nothing to restart.

The approval: spawning an agent on a paid route already asks the person (`consent` names the model); it gains the route, the key's label and the per-day cap. "Always" for an agent and a route is the Vault's existing grant, `route:<id>`. A person creating an agent in the form approves by creating it.

## 5. The MuxAPI surface

### 5.1 `Agents` spawn, create, update

New parameters on spawn (single and `workers`), create and update: `harness` (default: the leader's for spawn, the template's for create), `route`, `model`. `runtime` and `container_image` stay. Validation is one function used by the MCP tool, the HTTP routes and the New agent form, so the three cannot drift: the route must serve the harness, the model must be one the route serves, the key must exist (or the answer says the person will be asked), and a paid route needs the consent above.

`Agents` gains `routes`: the catalog as the caller may use it, one row per route and harness (id, label, models with context and price, whether its key is present, whether it is paid). Read-only; available to every agent, container agents included, as the operator's container rule asks.

### 5.2 `harness` on spawn

Today a worker is a copy of the leader's definition, so a Claude Code leader cannot spawn a Codex worker, and a Codex leader cannot spawn Claude Code on DeepSeek. `harness` makes the worker's harness a choice. Its startup file, memory delivery and CLI install follow the chosen harness as they do for any agent of that harness (`agent_config.rs`, `memory_delivery.rs`).

### 5.3 The person's form

The New agent form gets Harness, Route and Model, in that order, filled from `routes`. The same refusals appear as words under the field.

### 5.4 The operator-only guard

`model_vendor_base_url` stays operator-only: a free URL receives every prompt and a key. A *catalog route* is not a free URL: AgentMux owns its address, so an agent choosing `openrouter` cannot send traffic anywhere the operator did not put in the catalog. That is consistent with the rule that agents may do anything but forge the operator's voice. Adding a route to the catalog is the question (D5).

## 6. What degrades on a non-Anthropic model

Marked **read** (stated by OpenRouter's guide today), **code** (from our code) or **to test** (I did not run it, and the build's first PR runs it once on a real route).

| Area | What I expect | Basis |
|---|---|---|
| Tool use | Works; all endpoints of both DeepSeek models support tools. Quality of tool calls varies by model and provider. | read |
| Thinking | OpenRouter lists thinking blocks as supported. A session started on Anthropic and resumed on another model carries Anthropic thinking blocks with signatures; whether the other backend accepts them is untested. | read; **to test** |
| Prompt caching | Passed through where the endpoint supports it (OpenRouter lists cache-read prices for both models). Hit rate and cost differ per provider. | read |
| Images | Depends on the model; the route carries a capability flag and the harness is told. DeepSeek's V4 models' image support is not confirmed here. | **to test** |
| Anthropic server-side tools (web search, web fetch run by Anthropic) | Likely absent on another backend. | **to test** |
| Context size | 1,048,576 for both DeepSeek models; the `[1m]` suffix keeps Claude Code from compacting early. | read |
| Cost shown by Claude Code | Claude Code prices by model name; for an unknown model it can show zero or a wrong figure. We show our own from the gateway (section 9) and stop trusting the CLI's figure for a route. | **to test**; code (`ambient/spend.rs`, the pane's usage line read the CLI's) |
| Plan limits and "usage limit" states | Do not apply: they come from an Anthropic sign-in's plan (`usage_limit` in `AgentRuntime`). A route has credit and a cap instead. | code |
| Account and switch logic | Assumes a Claude login (`needs_login`, `account`, the switch-account flow). A route agent has no account: the launch gate must not block it, and the Swarm must not offer a Claude account switch for it. The build checks `identity/resolver/inject.rs` layer 3 and `agent_lifecycle::start_blocked`. | code; **to test** |
| Which model the pane shows | The stream's init message names the model Claude Code was told, not what answered. The pane shows the route's id and the gateway's `model` from the response, so a mapping that sends Sonnet to DeepSeek is visible. | code |
| Fast mode, Fable-only options | Not offered off Anthropic's own models (OpenRouter's guide). | read |

The first PR of the build runs one real session on the route (a short task) and writes the table's "to test" rows from it. That is a feature test of the change, not a lab; any benchmark or soak beyond it is asked of the operator first, as the operator's rule of 2026-11-06 says.

## 7. The same model gives "switch the model in place"

Changing `model_route` and `model` of a running agent (the panel, defaults and edge cases are in 7a) is: record them, stop the harness at a turn boundary, relaunch with the same session resumed (`--resume`), on the new route. Report #24714 verified the underlying step: a session resumed with a different `--model` kept its conversation. `Agents update` takes `route` and `model`, and a person makes the same change in the composer's model/effort panel (7a); there is no Model control on the agent card. `Agents compact`/`fresh` are unchanged. Two cautions: a resume that crosses to another provider may be refused for the thinking-block reason in section 6, in which case the update says so and offers "Compact, then switch" instead of failing silently; and a switch while a tool call is running waits for the turn to end.

## 7a. The model panel: any model on a fixed harness

The operator's question, 2026-11-06: how much of the architecture is disrupted by changing the model on the fly to any model at all ("essentially the model panel on a claude agent … moves from just selecting claude/sonnet, to all our supported models"), with the harness fixed, and with the default-model and edge cases considered. Yes, it is part of the same work as the model switch of report #24714, done in the panel that already exists (not a new chip on the card) and extended from Claude's own models to every route that serves the harness. The short answer to "how much is disrupted" is the table in 7a.4: little, and in four places.

### 7a.1 The panel

The panel exists: the composer's `AgentRuntimeDropup` (Mode · Model · Effort). The model switch lives here and nowhere else; the card and lists show the result (section 11). Its Model options come from `getProvider(id).models` overlaid by the live `providers.models` catalog (for Claude: Opus, Sonnet, Haiku, Fable; for Codex: the models the account offers). What a pick does today: it is saved with the agent (`last_runtime`, block meta), and the "agreement" line (`runtime-dropup-agreement.ts`, `process-runtime.ts`) says when the running process differs from the selection until the relaunch lands. The build confirms in P3 whether a pick relaunches by itself or only marks the drift.

What changes:

- **Grouped by route.** The Model section lists every model of every route that serves this harness, a group per route: *Anthropic (your plan)*, *OpenRouter*, later *DeepSeek*. The harness never changes in this panel: it lists no other harness's models.
- **Each row** shows the label, context size, price in and out per million tokens (or "your plan"), capability marks (tools, thinking, images), and whether the route's key is present.
- **An entry that cannot be used is greyed with the reason**, and cannot be picked: "key missing: add it in the Vault", "out of credit", "context 128K is smaller than this conversation (210K)", "can't read images (3 in this conversation)", "this route isn't approved for this agent".
- **The stored value** is route-qualified (`openrouter:deepseek/deepseek-v4.1-flash-1017`). A plain `sonnet` or `claude-…` is read as the Anthropic route, so every value stored today stays valid. (Today `Remembered::parse` drops anything that is not `opus`, `sonnet`, `haiku` or `claude-…`; that filter is one of the places that change.)

### 7a.2 Defaults

There are three kinds of choice, and the panel says which one an agent has:

| Choice | Stored as | Follows | Changes under a running agent? |
|---|---|---|---|
| **Default** | nothing (or `default`) | the harness's, or the route's, default model. For Claude it is the `default: true` row, `sonnet`, which must equal `DEFAULT_RUNTIME_CONFIG.model` (a mismatch is a known trap, `catalog.ts`). | Never mid-session. A new default applies at the next launch. The panel shows "Default (Sonnet 5.5)" with the current name. |
| **Family alias** | `opus`, `sonnet`, `haiku` | the CLI's latest in that family | Same: resolved at launch. |
| **Pinned id** | a concrete id | nothing; it stays | Only when it is retired. |

- **A default changing under a running agent** does nothing until the agent next starts; the agreement line may say "running Sonnet 5.4; the default is now 5.5, applies at the next start", as information and not as a warning.
- **A pinned model retired** (the route's list no longer has it, or the upstream answers that it is gone): today `useModelMigration` moves a stored id to its family's successor, and for an account-listed provider (Codex) to the list's default. With routes the rule is: migrate **within the same route only**; if the route has no successor, fall back to that route's default and tell the person once in a pane card ("deepseek-v4.1-flash-1017 was retired by OpenRouter; this agent now uses …"). It never moves an agent to another route or vendor by itself: that changes who is paid and who sees the prompts.
- **Workers spawned by a leader** start on the leader's remembered runtime with the spawn's `model` over it (`worker_runtime`). With routes: the worker inherits the leader's route and model unless `route` or `model` is given. A leader on a paid route that spawns workers on it does so under the spawn consent (section 4), which names the route and the day's cap.
- **Template defaults:** a template's remembered runtime is the default for agents made from it. If that route is not available on this machine (no key, route not approved), the form falls back to the harness's default route and says so.

### 7a.3 Edge cases, and what each does

| Case | Behaviour |
|---|---|
| **The transcript is bigger than the new model's context** | The panel compares the conversation's current context size (the CLI's usage report; `context-window.ts` already knows each family's window) with the new model's context. If it fits with room to spare, switch. If not, the row offers "Compact, then switch" (`Agents compact` exists) as its action. If even the compacted conversation does not fit, the row is greyed with that reason. Nothing is truncated silently. |
| **Images in the history, the new model has no vision** | The panel counts the images in the conversation and says "3 images; this model can't read them". The harness's behaviour with an image block sent to a text-only backend is untested; the gateway turns an upstream refusal into one clear message, the switch is undone (next row), and compaction (which turns images into text) is offered. |
| **Thinking blocks across providers** | Anthropic's thinking blocks carry a provider signature, so a backend that is not Anthropic may refuse a history containing them (to test, section 6). Rule for any cross-provider switch: **the new model is committed only after its first turn succeeds**. If that turn is refused for the history, the previous route and model are restored, the person sees a card, and "Compact, then switch" is offered, since a compacted conversation has no thinking blocks. |
| **Switching mid-turn or mid tool call** | The pick is saved at once and **applies at the next turn boundary**; a tool call in flight is never killed. The panel says "applies after this turn". "Switch now" is a separate action that interrupts, as Stop does. |
| **Prompt-cache loss** | A cache is per model, so a switch re-reads the whole conversation at the new model's input price. The panel shows the cost before the switch when the conversation is over about 50K tokens: "about $0.09 to re-read 180K tokens on this model", from the route's price table. |
| **Login route to key route, and back** | Leaving a login route (a Claude plan) means plan limits, the `usage_limit` state, the account-switch offer and failover between accounts stop applying: a key route has credit and a cap instead. When the login route hits its limit, the panel **offers** (never takes) a key route: "Sonnet is at its limit until 3pm. Continue on DeepSeek via OpenRouter?" Automatic failover to a paid route is a spend decision, so it is off (D11). Going back needs a signed-in account; if it is itself at its limit, the usual `usage_limit` state shows at once. |
| **The key is missing or out of credit at switch time** | The row is greyed with the reason and cannot be picked. If it becomes so between the pick and the first turn, the switch fails validation, the previous model stays, and the pane gets the Vault request card or the credit card of section 4. |
| **Role mapping (small and fast, subagent) when only the main model changes** | Roles belong to the route. Changing the main model within a route leaves the roles as they were; changing route sets all three from the new route's defaults, never carrying the old route's. Claude Code uses its small model for background work (titles, summaries), so on a key route that traffic is paid too: the panel's Advanced line shows the three roles and their models. |
| **An agent switching its own model through MuxAPI, and the cap** | Allowed (`Agents update` with `route` and `model`, on itself), applied at the next turn boundary. A switch **to a paid route** needs the same consent as a spawn on it unless the person has said "Always" for that agent and route (the Vault grant `route:<id>`). The day's cap applies to the agent whichever route it is on, and a thrash guard limits an agent to 6 switches an hour, since each costs a cache (D12). One agent switching another's model is the same call with the same consent. |
| **A container agent** | The same panel and the same call. The harness relaunches inside its container with its session file in the mounted config directory; the gateway token is already there; no key enters the container. The image needs the CLI, as it does now. |
| **Switching back** | The previous route and model are remembered per agent (`previous`), and the panel's first row is "Back to Sonnet 5.5". The same rules apply: the cache cost is shown again, and the first-turn commit applies. |

### 7a.4 What the change touches

| Subsystem | Change | Size |
|---|---|---|
| Launch environment (`launch_env`, `resolve_vendor_env_override`) | One function builds the environment from `(harness, route, model, gateway token)`, replacing the single-variable override. This is the `LaunchPlan` of section 8. | Medium |
| Identity inject and the launch gate (`identity/resolver/inject.rs` layer 3, `agent_lifecycle::start_blocked`, `needs_login`) | A key-route agent must not be stopped by "no signed-in account"; accounts stay for login routes. The place most likely to hide a surprise. | Small to medium |
| Quota and `usage_limit`, the Swarm's account switch | Apply only on a login route; a key route shows credit and cap. | Small |
| Model catalog (`model_catalog.rs`, `providers.models`, `setProviderModels`) | One catalog per route, cached per route; today's Anthropic and Codex lists become two routes. | Medium |
| Remembered runtime (`Remembered::parse`, `last_runtime`, block meta) | Accepts route-qualified ids; today it rejects anything but Claude's. | Small |
| Spend display (`ambient/spend.rs`, the pane's usage line) | From the gateway for a key route; unchanged for a login route. | Medium |
| The gateway | New. | Large (P1) |
| The picker (`AgentRuntimeDropup`, `runtime-dropup-*`) | Grouped Model section, greyed reasons, the cache-cost line, the Back row, the Advanced roles line. | Medium |
| **Not touched** | The agent's id and definition, the transcript store and every history reader (the raw stream keeps whatever model names it has), Personal and Global Memory, panes, tabs and layout, Message, Presence, Tasks, the Vault's internals, the container runtime and images, bundles and instruction sets, seats. | none |

So the architecture is not disrupted at the level of the agent: its identity, history and memory do not move, because the harness is fixed. The change is concentrated in how a launch is described (one function), who is allowed to launch (one gate), what the model list is (per route), and one new component, the gateway.

## 8. Not in this spec: switching the harness

The harness is fixed for an agent's life (section 1). Moving an agent to another harness is a later piece of work, not decided here, and nothing in this build touches it. The pieces it would reuse from this spec are the `(harness, route, model)` launch description and the route's `harnesses` list; the handover and the searchable transcript file are in report #24714 (arm D).

## 9. Billing and safety

- **Spend per agent** is counted in the gateway from the usage in each response, times the route's price table. It is shown on the agent card and in the Agents list ("$0.42 today"), and in the Swarm row as a small figure. For an Anthropic login route it stays as it is today.
- **A cap per agent per day**, default $5 for a route agent (D6), warns at 80% with a pane card and stops the gateway for that agent at 100% (the process stays; the next turn gets one clear refusal). A leader may raise it for a worker it spawned; a person may raise any.
- **A key-level cap at OpenRouter** is recommended in the Vault entry's notes, because our cap counts what passes through us and not other uses of the same key.
- **A kill switch:** `Agents update` with `route: ""` and the Vault's revoke both end an agent's access without a restart.
- **Logs** carry the route, model, token counts and cost, never the key, a prompt or an answer.

## 10. Container as the default runtime

- **New agents default to `container`** for spawn and create when Docker is available, by API; `native` is the explicit word. The New agent form already preselects a container when the template or the person's last choice says so and Docker and the image answer (`defaults/container-default.ts`); the change there is to make a container the template default, not to add logic. A container agent that cannot be started (no Docker) is refused with the words "Docker isn't available; pass runtime: native to run on this machine", never switched silently (the `agent_type` default's own comment, kept).
- **Existing agents keep theirs.** No migration. A per-user setting `agents:defaultruntime` (default `container`) exists for people who want the old default back.
- **A container agent on a route works with no credential in the container:** it carries the scoped token and reaches the gateway through the route it already uses for srv (`AGENTMUX_LOCAL_URL`).
- **Still native:** anything that reads or writes the host's files outside the container's mounts, runs host programs, drives the host's desktop, or uses the host's own `Terminal` — the operator's rule of 2026-10-28. Also a harness whose CLI has no container image yet (the build lists which of the 19 do).
- **The leader stays native** for now (the operator, 2026-10-28: "the leader should be running native, usually"); this default is for workers and new agents.

## 11. What the agent card and the lists show

The agent card gets one line: *Claude Code on DeepSeek V4.1 Flash via OpenRouter*, with the route's icon, the model's context size and today's spend. The Agents list and the Swarm row show the same as a read-only chip; changing it is done in the composer's panel (7a), not here. Limits and quota show the plan for a login route and the credit and cap for a key route, so the two are never confused. Memory, history and Personal Memory are unchanged: they follow the agent's id, not its route.

## 12. Decisions for the operator

The first four are the open decisions of report #24714, folded in so they are answered once.

| # | Question | Recommendation |
|---|---|---|
| D1 | **What "switch" means.** One agent, one id, one history, with the model or harness changing underneath (not a new thread, as AgentSky does for a harness change) | Yes |
| D2 | **Which harnesses get routes first** | Claude Code first (this spec), Codex second, then Gemini; the rest on demand |
| D3 | **How the key reaches the model:** the srv gateway (key never leaves srv), or the key in the harness's environment | The gateway |
| D4 | **A hosted cloud** (AgentSky's cloud computer with credits) | Not built; the container default and routes cover "any model, any machine". Your call on whether to host. |
| D5 | **Who may add a route to the catalog:** only AgentMux (built-ins), the operator in Settings, or an agent through MuxAPI with the key's Vault entry named | The operator in Settings, and an agent through MuxAPI when the person approves it in a card (a route sends every prompt to its address, so each new address is approved once) |
| D6 | **A daily cap per route agent**, and its default | On, $5, warn at 80% |
| D7 | **Container default** for new agents when Docker is there | Yes, with the per-user setting to turn it off |
| D8 | **`harness` on spawn**, so a worker can run another harness than its leader | Yes |
| D9 | **Messengers** (Slack, Teams, Telegram, Discord) | Pick one or two; not part of this build |
| D10 | **The DeepSeek direct route** (`api.deepseek.com/anthropic`) in the first build, or OpenRouter only | OpenRouter first; DeepSeek direct once OpenRouter works, since its key is a second Vault entry |
| D11 | **Automatic failover** from a login route at its plan limit to a paid key route | Off: offer it, never take it (it spends money) |
| D12 | **A thrash guard** on an agent switching its own model (each switch loses the prompt cache) | 6 switches an hour |

## 13. Build plan (the build task follows approval)

| PR | What | Effort (agent-days, my estimate, not measured) |
|---|---|---|
| P1 | Route catalog and the gateway: the `openrouter` route, the Vault lookup, the scoped token, streaming pass-through, spend counting; one real session on `deepseek/deepseek-v4.1-flash-1017` to fill section 6's "to test" rows | 5–7 |
| P2 | `harness`, `route`, `model` on spawn, create, update and the form; `Agents routes`; the shared validation; consent text; the launch gate for a route agent | 3–4 |
| P3 | The composer's model/effort panel grows to every model on the routes that serve the harness (7a): grouped Model section, greyed reasons, defaults, the edge cases, the cache-cost line, Back; the in-place switch behind it. The card and lists get no control. | 4–5 |
| P4 | Container default and the per-user setting; the list of harnesses without an image | 2 |
| P5 | Spend on the card and lists; the cap and its cards | 2–3 |
| later | DeepSeek direct, Bedrock, Vertex; Codex routes | per item |

Every PR follows the usual checks (the change gates, a look check on macOS and Linux for the form and card changes).

## 14. What I did not do

I did not run Claude Code against OpenRouter or DeepSeek (no key is in the Vault yet), so section 6's "to test" rows are expectations, not results. I did not open DeepSeek's own documentation (the direct address came from a search result). I did not read the gateway's streaming behaviour against OpenRouter's, which P1's first session tests. The effort figures are guesses.

## 15. As built: P3a (the switch in srv)

P3 is built in two PRs, because the panel needs srv's half first. P3a is the half with no screen.

- **Where the route lives.** The route and model stay the agent's content blob (`agent:model_route`), and the pane's `agent:runtime.model` stays the harness's own alias. So `Remembered::parse` does not change: a route-qualified id is never stored there, and the gateway maps any model name the route does not serve to the agent's chosen one. This replaces the "stored value is route-qualified" line of 7a.1.
- **`model_gateway::switch`.** `move_to` validates (route serves the harness, model served, key in the Vault), keeps the old place as `previous` (the panel's Back row; content `agent:model_route_previous`, written only by srv), writes or clears the choice, and takes the left route's gateway token away. `agent_switch_allowed` and `count_agent_switch` are decision D12: an agent changing its own model is held to 6 an hour, counted once the change is made so a refused one costs none; a person's click is not counted.
- **`Agents update`** now goes through `move_to`, so it records `previous`, counts against the hour and restarts the agent's panes at the end of the turn they are in (the same restart an account switch uses).
- **Two commands for the window only** (`ctx.operator_only`: the click is the approval of the spend): `agent.model.panel` returns the routes serving the pane's harness with each model's context, prices and capabilities, which one is current, the previous place, and for each row why it cannot be picked ("key missing: add \"OpenRouter API key\" in the Vault", "can't read images (N in this conversation)") and, over 50K tokens of conversation, what re-reading it costs at that model's input price. `agent.model.switch` moves the agent (route `""` is the harness's own sign-in) and restarts its panes unless told not to.
- **The real model on the pane.** At every launch srv writes `agent:route_model` ({route, route_label, model, label}) on the pane's meta, or clears it off a route. The panel, the composer footer and the pane header's model icon (task #29322) read that one field, so they agree.
- **Not in P3a:** the panel (P3b); committing a cross-provider switch only after its first turn succeeds, and "Compact, then switch" (P3c); an agent putting itself on a paid route with a "Always" grant (an agent still cannot leave its route for another: a person does it in the panel).

## 16. As built: P3b (the panel)

- **Where.** The composer's runtime panel (`AgentRuntimeDropup`, `runtime-dropup-*`) keeps its Mode and Effort sections. Its Model section is now a Back row (when `agent.model.panel` reports a `previous`), a "Your plan" group with the harness's own models, and one group per route serving the harness ("billed to your key in the Vault", or "key missing"). Only a Claude Code pane asks srv for routes; any other harness, or an srv without the command, shows the harness's own panel as before.
- **A row** names the model and says under it: context, price in and out per million, and what it does (tools, thinking, images), with the cost of re-reading the conversation on that model when it is over 50K tokens and not the model already running. A model srv says cannot be picked (key missing, cannot read the images in the conversation) is greyed with srv's reason, and kept out of keyboard navigation.
- **A pick** calls `agent.model.switch` (`restart: true`): srv records `previous`, writes the choice and restarts the pane at the end of its turn, with its conversation. Picking a harness model while on a route first leaves the route (`route: ""`, `restart: false`) and then sets the runtime as it always did, which restarts it once. A failed switch is shown on the trigger and leaves the agent where it was.
- **The footer.** The trigger's label shows the model that really answers (`agent:route_model` on the pane, written by srv at each launch), not the harness alias Claude Code reports; the line under the Model rows reads "Running: GLM 5.3 on OpenRouter". Off a route nothing changes.
- **Not in P3b:** committing a cross-provider switch only after its first turn succeeds, and "Compact, then switch" (P3c).

## 17. As built: P3c, part one (the image count in srv)

- **Where the count comes from.** `backend/transcript/images.rs` counts the images in an agent's conversation from its transcript (zone `agent:<definition id>:current`), not from the pane's loaded feed, so images in history the pane has rolled off or not yet loaded count too. It reads the `<attached_files>` list at the end of each user message: a found image is a path ending `.send.<image extension>`, a missing one a `- name.png — (no longer available)` line. Only user lines are read; an agent's own text quoting a list counts for nothing. A repeat count reads on from where the last stopped.
- **The panel** takes the larger of the pane's `images` and srv's count, so the frontend count is a hint and can only add. **The switch** (`agent.model.switch`) refuses a route model that cannot read images when the conversation holds one, with the same text the panel greys the row with ("can't read images (N in this conversation)").
- **Not counted:** images carried by a jekt or an "Earlier message" note (the pane's hint still counts those), and images only in the provider's own session file after a compaction.
