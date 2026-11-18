# Spec: an Anthropic API key as an account in the agent pane's menu (first slice)

**Status:** active. First slice (parts B.1 to B.3, Anthropic only) of task #47858, built in #47859; container agents (section 5) built in #47887. Author: Agent5@narko, 2026-11-16.

## Requirement

An Anthropic API-key account added in Connectors > Accounts (provider `anthropic`, kind `api_key`, with a display name) appears in the same account menu of the agent pane's composer chip as the Claude plan accounts, under its display name. Picking it moves a Claude agent onto the key exactly as a plan switch does.

## Behaviour

1. **Menu.** Lists the Claude plan accounts and the Anthropic API-key accounts. A key row shows its display name (the account name when it has none), and the mark `API` that keeps its width however long the name is, so it cannot be mistaken for a plan. Tick, the `·N` count suffix, the queued and preparing notices and the switch itself behave as for a plan. A key that was saved without a check (status unknown) is offered; one that is known invalid is not.
2. **Switch.** Keeps the conversation after every move (plan to key, key to plan, key to key), as a move between plans does: the agent restarts, resuming its session when that is reachable from the folder its new payer runs in (a key agent runs in the provider's default folder, so a restart on a key, or a move to another key, resumes the same session), and otherwise carrying AgentMux's record of the conversation into the new session. The agent has exactly one payer: moving to a key removes the Claude plan link, moving to a plan removes the key link. The chip then shows the key's display name and the `API` mark.
3. **No silent change of who pays.** There is no automatic move from a plan to a key or from a key to a plan, or from a key to another key. A pane on a key that hits a limit, a rejected key (401) or used-up credit stops with the existing unavailable failure and says so; the chip menu stays available. The failover and the login-expiry move never pick a key account, and never move an agent off one.
4. **The agent cannot read the key.** The key stays in the keychain. The agent's CLI is pointed at AgentMux's model gateway with a token that is useless anywhere else, and the gateway adds the key on the way to Anthropic. No tool, file, environment variable or process listing of the agent holds the key, and its value is never shown, logged or put in a transcript, task, message, URL or command line. The token is for one agent and one account, is accepted only from this machine, and stops working the moment the agent is moved off the key. The gateway counts the key's use per agent per day and stops at the agent's daily cap (the same default as any gateway route) with a plain billing message, never later than the dollar figure it states: a cache write counts at its real rate (1.25 times the input price for five minutes, 2 times for one hour) and at the dearest rate when the usage report does not say which. Only a Claude agent's key goes through the gateway, so only a Claude agent on a key has a spend card and a cap that can be read, raised and reset; requests pass through unchanged (prompt caching included).
5. **Container agents** are offered the same accounts as native ones, keys included, and a switch is the same switch. The key never enters the container (not its environment, files, mounts, image, process list or logs): the container is given only the gateway's address and a token for that one agent and that one account, and no sign-in of the person's is mounted for it.
   - **The token** is made current at the start of each turn: a container made again, a new address or another account gets a new one and the earlier ends, while a turn that overlaps another of the same agent (two panes) shares its token, so neither is cut off mid-turn. It ends when the container is stopped, recreated or deleted, when the agent is deleted, and the moment the agent is moved off the key.
   - **What it holds for, per platform.** On Linux the token works only from that agent's own container: it is tied, at each turn start, to the container's own address on AgentMux's agents network, and a request that shows it from any other peer is refused, recorded in the Audit view (the agent and the key's display name, never the token) and ends the token at once, so the agent's next turn gets a new one. On macOS and Windows the token works only from this machine.
   - **No path, no fallback.** Where AgentMux cannot open the gateway to the container's network on Linux (rootless Docker, for one), the agent's turn says so and says what to do: run the agent on this machine, or move it to a plan account. It never falls back to another payer.
   - **No raw sockets.** An agent's container does not get the raw-socket capability (`NET_RAW`), so tools that need it, such as `ping`, may not work in it. A container made before this is recreated once.
   - An agent on a model route is billed to the route and is not offered a key, and the other way round: an agent on a key is not put on a model route from the model panel, so there is one payer.

## Not in this slice

Other providers' keys (B.4), fleet moves (B.6), what an account has left (part C).

## Tests

- Menu: a key account appears with its display name, `API` mark (kept whole beside a long name); a plan does not carry the mark; the tick is on the current one; an invalid key is not offered; a container agent is offered the same rows as a native one.
- Chip: shows the key's display name and mark when bound to a key.
- srv switch: a Claude agent can switch to a key account and back; exactly one link remains; panes restart; an unchanged switch reports unchanged; a container agent takes a key and leaves it as a native one does; refused for an agent on a route; refused for a key known invalid.
- Spawn gate: a Claude agent linked only to a key spawns (the gate is satisfied) with the gateway address and its token and no key anywhere in its environment; the gateway adds the key upstream; the key text is in no log line, response or audit row; a token stops working when the agent is unlinked.
- Gateway: a rejected key, a used-up credit and a cap reached reach the agent as errors naming only the account's display name; the request body and the caching headers go through unchanged.
- Failover: a limited pane on a plan never targets a key; a limited pane on a key is never moved.
- Conversation: a restart in the folder the key route uses resumes the same session; a session under another login's folder is not adopted and the record of the conversation is carried instead; the key route's folder is set at spawn.
- Spend: a cache-heavy response counts at the real write rates and, unseparated, at the dearest; a non-Claude agent holding an Anthropic link has no cap; a key agent's cap can be read, raised and reset.
- Redaction: the key value does not appear in the switch result, audit row, notice or log lines.
- Container agents: a container agent linked to a key is given the gateway's address and a token and no key; no login of the person's (credentials file or Keychain) is mounted for it, and its memory folder still is; each turn's token is new and the earlier one ends; the token is bound to the container's address on Linux and accepted from no other peer (a foreign peer is refused, recorded by agent and display name without the token, and the token ends); the token ends when the container is stopped, recreated or removed and when the agent is deleted or moved off the key; Linux with no gateway on the agents network refuses the turn with what to do; an agent container drops `NET_RAW`, and one made without it is recreated; the gateway address a model-route or key-route agent is given follows the listener when it comes up on the first turn.

## Look checks

macOS (Clare, Masty, Maop) and Linux (Maricon, Opaz, Parlo): the menu with a plan and a key row, for a native and for a container agent; the chip with a long display name, wide and narrow (the mark survives the cut); the menu at the window's minimum width with a long display name (the menu stays inside the window); light and dark.

## Proof

A debug `task dev` build is started; a person adds their own key in that build's Connectors and switches an agent themselves. No agent types or handles the key. The steps: add the key; switch an agent that has a conversation to the key; send a message and see that the agent answers with the earlier conversation in mind (the conversation continues after each move); switch back to the plan and repeat; with two keys, switch from one to the other and repeat; check the agent's first turns for the cache hit rate the gateway logs. For a container agent: the same steps with an agent that runs in a container, then check from inside its container that no environment variable, file or process argument holds the key, and, on Linux, that its token does not work from another container on the agents network.
