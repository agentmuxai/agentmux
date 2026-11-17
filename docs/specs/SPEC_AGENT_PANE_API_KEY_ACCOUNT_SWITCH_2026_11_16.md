# Spec: an Anthropic API key as an account in the agent pane's menu (first slice)

**Status:** active. First slice (parts B.1 to B.3, Anthropic only) of task #47858, built in #47859. Author: Agent5@narko, 2026-11-16.

## Requirement

An Anthropic API-key account added in Connectors > Accounts (provider `anthropic`, kind `api_key`, with a display name) appears in the same account menu of the agent pane's composer chip as the Claude plan accounts, under its display name. Picking it moves a Claude agent onto the key exactly as a plan switch does.

## Behaviour

1. **Menu.** Lists the Claude plan accounts and the Anthropic API-key accounts. A key row shows its display name (the account name when it has none), and the mark `API` that keeps its width however long the name is, so it cannot be mistaken for a plan. Tick, the `·N` count suffix, the queued and preparing notices and the switch itself behave as for a plan. A key that was saved without a check (status unknown) is offered; one that is known invalid is not.
2. **Switch.** Keeps the conversation after every move (plan to key, key to plan, key to key), as a move between plans does: the agent restarts, resuming its session when that is reachable from the folder its new payer runs in (a key agent runs in the provider's default folder, so a restart on a key, or a move to another key, resumes the same session), and otherwise carrying AgentMux's record of the conversation into the new session. The agent has exactly one payer: moving to a key removes the Claude plan link, moving to a plan removes the key link. The chip then shows the key's display name and the `API` mark.
3. **No silent change of who pays.** There is no automatic move from a plan to a key or from a key to a plan, or from a key to another key. A pane on a key that hits a limit, a rejected key (401) or used-up credit stops with the existing unavailable failure and says so; the chip menu stays available. The failover and the login-expiry move never pick a key account, and never move an agent off one.
4. **The agent cannot read the key.** The key stays in the keychain. The agent's CLI is pointed at AgentMux's model gateway with a token that is useless anywhere else, and the gateway adds the key on the way to Anthropic. No tool, file, environment variable or process listing of the agent holds the key, and its value is never shown, logged or put in a transcript, task, message, URL or command line. The token is for one agent and one account, is accepted only from this machine, and stops working the moment the agent is moved off the key. The gateway counts the key's use per agent per day and stops at the agent's daily cap (the same default as any gateway route) with a plain billing message, never later than the dollar figure it states: a cache write counts at its real rate (1.25 times the input price for five minutes, 2 times for one hour) and at the dearest rate when the usage report does not say which. Only a Claude agent's key goes through the gateway, so only a Claude agent on a key has a spend card and a cap that can be read, raised and reset; requests pass through unchanged (prompt caching included).
5. **Container agents** never receive the key. Key accounts are not offered to a container agent; the menu says why ("API keys aren't offered to container agents yet"); srv refuses the switch with the same reason. The same refusal applies to an agent on a model route (it is billed to the route), and the other way round: an agent on a key is not put on a model route from the model panel, so there is one payer.

## Not in this slice

Other providers' keys (B.4), fleet moves (B.6), what an account has left (part C).

## Tests

- Menu: a key account appears with its display name, `API` mark (kept whole beside a long name); a plan does not carry the mark; the tick is on the current one; an invalid key is not offered; a container agent sees no key rows and the reason row.
- Chip: shows the key's display name and mark when bound to a key.
- srv switch: a Claude agent can switch to a key account and back; exactly one link remains; panes restart; an unchanged switch reports unchanged; refused for a container agent and for an agent on a route; refused for a key known invalid.
- Spawn gate: a Claude agent linked only to a key spawns (the gate is satisfied) with the gateway address and its token and no key anywhere in its environment; the gateway adds the key upstream; the key text is in no log line, response or audit row; a token stops working when the agent is unlinked.
- Gateway: a rejected key, a used-up credit and a cap reached reach the agent as errors naming only the account's display name; the request body and the caching headers go through unchanged.
- Failover: a limited pane on a plan never targets a key; a limited pane on a key is never moved.
- Conversation: a restart in the folder the key route uses resumes the same session; a session under another login's folder is not adopted and the record of the conversation is carried instead; the key route's folder is set at spawn.
- Spend: a cache-heavy response counts at the real write rates and, unseparated, at the dearest; a non-Claude agent holding an Anthropic link has no cap; a key agent's cap can be read, raised and reset.
- Redaction: the key value does not appear in the switch result, audit row, notice or log lines.

## Look checks

macOS (Clare, Masty, Maop) and Linux (Maricon, Opaz, Parlo): the menu with a plan, a key row and the container reason row; the chip with a long display name, wide and narrow (the mark survives the cut); the menu at the window's minimum width with a long display name (the menu stays inside the window); light and dark.

## Proof

A debug `task dev` build is started; a person adds their own key in that build's Connectors and switches an agent themselves. No agent types or handles the key. The steps: add the key; switch an agent that has a conversation to the key; send a message and see that the agent answers with the earlier conversation in mind (the conversation continues after each move); switch back to the plan and repeat; with two keys, switch from one to the other and repeat; check the agent's first turns for the cache hit rate the gateway logs.
