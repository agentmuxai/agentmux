# Sub-agent Models: general-purpose on Sonnet, Explore on Haiku

**Date:** 2026-10-25
**Status:** implemented — approved by the operator on 2026-10-25 ("proceed with recommendations") after the token-efficiency review. Built in #5458.
**Owner:** agent-pane lifecycle
**Related:** [`SPEC_AGENT_SELF_COMPACTION_2026_10_23.md`](SPEC_AGENT_SELF_COMPACTION_2026_10_23.md) (the other half of the token work)

---

## 1. Why

A Claude Code sub-agent inherits its parent's model. AgentMux's agents run on the largest model, so their sub-agents did too: in a week of transcripts sub-agents were about a quarter of all spend, and nearly all of their calls ran on the largest model. A search or a file read doesn't need it. Moving general-purpose sub-agents to Sonnet and Explore sub-agents to Haiku was estimated to save about a tenth of total spend.

## 2. What Claude Code does (measured on 2.1.288)

One tiny task per sub-agent type, a parent on Opus 5.5, `message.model` read from the stream:

| Case | General-purpose | Explore | Plan |
|---|---|---|---|
| nothing set | Opus | Opus | Opus |
| `CLAUDE_CODE_SUBAGENT_MODEL=sonnet` | **Sonnet** | Opus | Opus |
| `CLAUDE_CODE_SUBAGENT_MODEL=haiku` | | Opus | |
| an `Explore` agent definition with `model: haiku`, in the project or in the config dir's `agents/` | | **Haiku** | |

So the environment variable moves general-purpose sub-agents only; Explore and Plan ignore it. A definition named `Explore` replaces the built-in, and it wins over the variable. Without `CLAUDE_CODE_SUBAGENT_MODEL_FORCE`, a sub-agent's own `model:` and the `model` its parent passes per call still win, so a leader can send a hard question to Opus.

## 3. What AgentMux does

- **General-purpose.** At each Claude agent's spawn (`build_persistent_spawn_env`) it sets `CLAUDE_CODE_SUBAGENT_MODEL` from, in order: the pane's block meta `agent:subagentmodel`, the setting `agent:subagentmodel`, then `sonnet`. `inherit` (or `off`, `none`, `default`) sets nothing. An explicit `cmd:env` value wins. Other providers are left alone. The value must look like an alias or a model ID.
- **Explore.** When it prepares a Claude config dir it writes `agents/Explore.md` there from the setting `agent:exploremodel` (default `haiku`): read-only tools, a short search-specialist prompt, and a managed marker. A changed setting rewrites it; `inherit` removes it again. One a person wrote (no marker) is left alone. It takes effect when each agent next starts, like `agent:compactat`.
- **Plan** is left on the parent's model on purpose.
- **Per agent.** The pane's block meta overrides the setting for general-purpose sub-agents. Explore is per config dir (per account), because that is where its definition lives; the model passed per call is the per-agent lever there too.

Both settings are in Settings ▸ Advanced.

## 4. Risks

- A replaced Explore loses the built-in's own prompt. The replacement is short and read-only; if search quality drops, set `agent:exploremodel` to `inherit`.
- Sonnet and Haiku are weaker on hard research. A leader that needs the largest model for one sub-agent passes `model: "opus"` on that call, which this does not block.
- Savings depend on how often agents use sub-agents; the figure above is an estimate from one week.

## 5. Tests

- Resolution order, `inherit`, a model ID, and a value that isn't a model name (unit).
- Which providers get the variable, and that an explicit `cmd:env` wins (unit).
- The Explore definition: written, left alone when unchanged, rewritten on a change, removed on `inherit`, and never replacing a person's (unit).
- The probe above, rerun on a CLI upgrade.
