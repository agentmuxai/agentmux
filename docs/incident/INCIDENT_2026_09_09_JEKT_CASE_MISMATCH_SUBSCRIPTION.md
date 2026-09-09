# INCIDENT 2026-09-09 — investigated a claimed "claude" vs "Claude" WAN-subscription case mismatch; disproved it, reconfirmed #3100 is still open

**Status:** historical / research only — no code changed. The case-mismatch hypothesis that motivated this investigation is **disproven** by source citations below. The actual live risk in this session is the *same, still-unfixed* mechanism documented in `INCIDENT_2026_09_08_REAGENT_JEKT_NOT_DELIVERED.md` (merged as #3100) — none of its §6 options have been implemented since.

**Date:** 2026-09-09

## 1. The hypothesis this investigation was asked to check

A `DiscoverAgents` call in this session returned `host.agents`/`host.addressable` with `agent_id`/`name: "Claude"` (capital C, the live UI display name) alongside `wan.local_agents_subscribed: ["claude"]` (lowercase). The hypothesis: if the cloud relay or github-router matches an inbound jekt's target agent name against the WAN subscriber list *case-sensitively*, while local delivery normalizes case, a post-rename capitalization drift alone could explain "jekts never arrive, local messaging still works" — a distinct bug from #3100's `agentg`-vs-`claude` (different strings entirely, not a case variant of the same string).

## 2. Why the hypothesis is wrong

The case difference between `"Claude"` and `"claude"` in the `DiscoverAgents` payload is cosmetic and expected, not a bug. `host.agents`/`addressable` surface the display name as typed (from SQLite/live block meta); `wan.local_agents_subscribed` surfaces the *lowercased registration key*. The tool's own doc comment says so directly:

- `agentmux-srv/src/server/mod.rs:741` — "Addressing is case-insensitive (registration lowercases the key)."
- `agentmux-srv/src/server/mod.rs:746` — "Keep the live block_id per name (lowercased)..."

Every comparison point in the actual delivery path — both local (agentmux-srv) and cloud (agentmux-cloud/muxbus) — normalizes to lowercase before comparing. Checked directly, this session:

- `agentmux-srv/src/backend/reactive/handler.rs:192` — `register_agent`: `let agent_key = agent_id.to_lowercase();`
- `agentmux-srv/src/backend/reactive/handler.rs:384` — `inject_message`'s block lookup: `self.agent_to_block.get(&req.target_agent.to_lowercase())`
- `agentmux-srv/src/backend/reactive/handler.rs:431` — the recipient-identity check (#2695 mitigation) compares `actual_agent_id.to_lowercase() != req.target_agent.to_lowercase()` — both sides lowered, not just one
- `agentmux-srv/src/backend/reactive/handler.rs:859` — the async injection path: `let target_key = target_agent.to_lowercase();`
- `agentmux-cloud/muxbus/server/src/index.ts:103-105` — `normalizeAgentId(agentId) { return agentId.toLowerCase().trim(); }`
- `agentmux-cloud/muxbus/server/src/index.ts:389` — `/reactive/inject`: `const normalizedTarget = normalizeAgentId(target_agent);`
- `agentmux-cloud/muxbus/server/src/index.ts:579` — `/reactive/status/:injection_id`: same `normalizeAgentId` applied to both `source_agent` and `target_agent` before the caller-authorization comparison
- `agentmux-cloud/muxbus/consumers/github/agent-mapping.ts:107` — `getAgentId`: `const lower = githubUsername.toLowerCase();`
- `agentmux-cloud/muxbus/consumers/github/agent-mapping.ts:237` — `extractAgentIdFromBody` (the PR-body-tag fallback that actually resolved the target in #3100's incident): `return extracted.toLowerCase();`

No raw, case-sensitive `===`/`==` comparison of an agent identifier exists anywhere on this path in either repo, as far as this investigation found. `shared-infrastructure/github-router` was re-checked (`grep -rn "muxbus\|agentmux" github-router/lambda/`, no hits) and, consistent with #3100 §2's finding, is confirmed uninvolved in jekt routing at all — it fans out raw webhooks + posts to Discord, nothing more.

**Conclusion: case sensitivity is not, and could not be, the delivery-failure mechanism.** The `"Claude"`/`"claude"` difference visible in `DiscoverAgents` output is two different, both-correct views of the same underlying lowercased key, not two competing identity records.

## 3. What actually is still broken (reconfirmed, not new)

This session independently reproduces #3100's real mechanism, live, right now:

- `env | grep AGENTMUX_AGENT_ID` → `agentg` (the stable slug, meant for PR-body tags per `agent_config.rs`'s `build_mcp_config()` doc comment)
- `DiscoverAgents` → `wan.local_agents_subscribed: ["claude"]`, `host.agents[0].name: "Claude"` (the live display name, lowercased at registration)

Same divergence, same session-vs-tag mismatch pattern #3100 described. `git log --oneline 6a5b45cbd..HEAD -- agentmux-srv/src/server/agent_handlers/input.rs agentmux-srv/src/backend/reactive/handler.rs` (6a5b45cbd = the #3100 merge commit) returns **no commits** — none of #3100 §6's three options (register under `AGENTMUX_AGENT_ID`; detect-and-surface the divergence; make `/reactive/inject` report delivery failure) have been implemented. A jekt tagged `<!-- agentmux:agent_id=agentg -->` today would fail to reach this session for exactly the reason #3100 already documented, not for any case-related reason.

## 4. Proposed next step (not a decision, matches #3100 §6)

No new fix is proposed here — this investigation found no new bug. The existing #3100 §6 options remain the right place to look, most directly option 1 (register the reactive handler under `AGENTMUX_AGENT_ID` in `input.rs`'s Register-tail, paired with fixing the #2695/#2697 stale-identity check some other way so it doesn't regress) or option 3 (make `/reactive/inject` / `injectToAgent()` surface "no active subscriber for target_agent" instead of reporting `response.ok` on mere queueing). Whoever picks this up should start from `input.rs:706-734` and `agentmux-cloud/muxbus/server/src/index.ts`'s `/reactive/inject` handler (~line 368-460), not from anything case-related.

## 5. Evidence index

- Live this session: `mcp__agentmux__DiscoverAgents` (`host.agents[0].name: "Claude"`, `wan.local_agents_subscribed: ["claude"]`), `mcp__agentmux__WhoAmI`, `env | grep AGENTMUX_AGENT_ID` (`agentg`).
- Source, read directly, this session: `agentmux-srv/src/server/mod.rs:725-820` (discovery endpoint + its doc comment), `agentmux-srv/src/backend/reactive/handler.rs` (register_agent, inject_message, async injection path, recipient-identity check), `agentmux-cloud/muxbus/server/src/index.ts` (normalizeAgentId + both call sites), `agentmux-cloud/muxbus/consumers/github/agent-mapping.ts` (getAgentId, extractAgentIdFromBody), `shared-infrastructure/github-router/lambda/` (grepped for `muxbus|agentmux`, no matches).
- `git log --oneline 6a5b45cbd..HEAD -- agentmux-srv/src/server/agent_handlers/input.rs agentmux-srv/src/backend/reactive/handler.rs` — empty, confirming #3100's fix options are still unimplemented.
- Prior report: `docs/incident/INCIDENT_2026_09_08_REAGENT_JEKT_NOT_DELIVERED.md` (#3100).
