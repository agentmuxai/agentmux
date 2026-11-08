# Spec: curate the agent-pane harnesses: nine first-class doors, one general harness, the rest preview

**Date:** 2026-11-07 (task #29362; the build is #29363)
**Status:** draft — awaiting the operator's confirmation of the keep and demote lists (section 9).
**Author:** Agent5@narko
**Read from:** `origin/main` at `ad233144b` (v0.60.20). Facts about other products are marked where they were not verified.
**Builds on:** `SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md` (the model route and the gateway), the provider registry (`crates/srv/src/backend/providers.rs`, `providers/*.rs`) and the frontend catalog (`frontend/app/view/agent/providers/catalog.ts`).
**Related:** #29360 (cost on the card and lists), #29364 (the goose-or-pi spike, parked by the operator), #27459 P4 (container default and the list of harnesses without an image).

## 1. The ask, and the answer in one page

The operator's direction, relayed by AgentY on 2026-11-07: curate the harness set. Today there are 19 providers; keep about 9 or 10 first-class, and keep the rest as an installable preview tier, not delete them.

**The answer.**

- **Eight first-class doors**, each the lab's own harness and the only way its flat-rate subscription is plainly worth buying: `claude`, `codex`, `gemini`, `copilot`, `grok`, `muse`, `qwen`, `kimi`. Antigravity collapses into Gemini (section 3).
- **One general harness** for every bring-your-own-key and OpenRouter model: `goose` or `pi`, **left open** until #29364 is resolved (section 4). This spec does not wait for it.
- **Eight preview**: `aider`, `cline`, `crush`, `opencode`, `kilo`, `openclaw`, `dsh`, `muxcode` (and `antigravity`, collapsed). Still installable and still working for an agent that already runs on one; just not preinstalled, not first in the picker, not in the defaults.
- **The mechanism is one field**, `tier`, on the provider in the registry and in the catalog, read by the picker, the container images and the docs (section 5). Nothing is removed from the code.
- **Why flat-rate decides the line** (section 7): a lab's subscription usually buys flat-rate access only through the lab's own harness. Another harness signed in to the same subscription can bill per token as "extra usage". This is also what the cost feature (#29360) must show.

## 2. What exists today (read from `origin/main`)

- **19 providers** in the registry, mirrored in `catalog.ts`: claude, codex, muxcode, gemini, qwen, openclaw, kimi, copilot, pi, antigravity, kilo, cline, opencode, goose, grok, dsh, aider, crush, muse. A provider is a `ProviderConfig` (`providers.rs`): launch args, resume flag, auth isolation, pinned npm package or managed install, supported vendors, the startup instructions file.
- **No tier field.** Every provider is equal in the code. The "New agent" panel offers them all; some carry a Free or Local tag (`provider-access.ts`, `provider-access-model.ts`).
- **Install is already per harness**: npm packages with a pinned version, or a managed download (antigravity, goose), and a one-click install for the vendor-installed CLIs (task #138). So "installable, not preinstalled" needs no new installer.
- **Hide and unhide of seeded picker templates exists** (`Agents hide`, `unhide`), and `reseedagents`. Demotion must not rely on it: hiding is a per-person choice about the picker, not a property of the harness.
- **Account binding differs.** Gemini binds per account (`GEMINI_CLI_HOME`); Antigravity's `agy` signs in once per machine and reads no per-account directory, so there is no account to bind (comment in `catalog.ts`). Account switching and failover (`agent.account.switch`) therefore work for Gemini and not for Antigravity.
- **The model route** (the model-routes spec) lets Claude Code run on OpenRouter models through srv's gateway, billed to a key in the Vault.

## 3. The keep list, and antigravity into gemini

| Harness | Door | Notes |
|---|---|---|
| claude | Anthropic (Claude plan) | also the harness for model routes |
| codex | OpenAI (ChatGPT plan) | |
| gemini | Google (Gemini CLI; Google account) | **the Google door** |
| copilot | GitHub (Copilot plan) | |
| grok | xAI | |
| muse | its lab's plan | |
| qwen | Alibaba | the OAuth free tier was retired; the harness stays for the key and the open models |
| kimi | Moonshot | no native startup-instructions file today |

**Antigravity into Gemini, which one stays: Gemini (recommendation).** Both are Google doors. Gemini CLI binds per account (so account switch and failover work), is on npm with a pin, and takes the Google account's access. Antigravity needs no per-account directory (nothing to bind or fail over), is a managed native download, and is the newer product. Keeping Gemini keeps the account features and the simpler install. **Not verified:** whether Google sells any flat-rate access that applies only through Antigravity; if it does, the operator may prefer the opposite, and the same mechanism supports it (flip which one carries `first_class`). Antigravity's existing agents keep working as preview (section 8).

## 4. The one general harness, left open

One harness for the models that have no lab harness of their own (any raw API, any OpenRouter model). Candidates: `goose` and `pi`. The slot is named **general harness (TBD: goose | pi)**; the registry field and the picker treat it as "first-class, the general slot", and it is filled by changing one tier value when #29364 is done.

Facts for whoever decides, none of which this spec relies on:

- QTest's finding: `pi` has no MuxAPI tools at the shipped pin (`pi-acp` 0.0.34). An agent without MuxAPI cannot message, spawn or claim tasks, which is much of what an AgentMux agent is. This favours goose.
- `goose` has no sign-in of its own (`goose configure` picks a model provider and key; its first session fails naming `GOOSE_PROVIDER` until then).
- **The model-route work changes the question.** Claude Code on an OpenRouter route (the model-routes spec) already runs any model on the full MuxAPI toolset, with the key in the Vault, never in the agent. That covers the Anthropic-protocol routes. The general harness is then needed for models and services that route cannot serve (other wire protocols), not as the default for BYO-key. This lowers the cost of leaving the slot open. (A question for the operator, D5: whether running a third-party model inside Claude Code is acceptable as a long-term default.)

## 5. The mechanism

One new field on the provider, written in the registry and the catalog (and kept equal by the existing parity test, `catalog-parity.test.ts`):

- `tier`: `first_class`, `general` or `preview`. `general` is the open slot: first-class in every behaviour, but named so the docs and the picker can say "the general harness" and not the product.
- `crates/srv/src/backend/providers.rs`: `ProviderConfig.tier`. Every `providers/*.rs` sets it. A test fails if two harnesses claim `general`, if there are more than 10 `first_class`, or if a provider has none.
- `frontend/app/view/agent/providers/catalog.ts`: the same value, checked against the registry by the parity test.

**What the tier changes** (and nothing else):

| Where | `first_class` and `general` | `preview` |
|---|---|---|
| New agent picker | cards at the top, in a fixed order | under a collapsed "More agents (preview)" section, each card tagged Preview |
| Container images | the harness is in the image (the list the container default P4 reads) | not in the image; installed on first use like today's vendor installs |
| Defaults and templates | may be a template's default; may be the "harness without an image" fallback | never a default |
| `Agents spawn` and `create` | as today | as today, with the harness named; the answer says "preview" |
| Toolchain pane | listed first | listed under Preview |
| Docs | the harness pages are written and kept current | one short page per harness: installable, not supported to the same depth |

**What it does not change:** an agent already running a preview harness keeps running, resumes, and keeps its history and memory; no stored record changes; the harness code, its translator and its tests stay. Preview means "not promised", not "removed".

**The picker.** The top section is the first-class cards plus the general slot; the second is collapsed by default and remembers its state per person. A search or the Free and Local filters search both. The exact component work (cards, the tag, the collapsed section, zoom, the look checks on macOS and Linux) is for Agent1 and Camper, with AgentX on the product direction; the build task names them.

## 6. The demote list

`aider`, `cline`, `crush`, `opencode`, `kilo`, `openclaw`, `dsh`, `muxcode`, and `antigravity` (collapsed into gemini). Notes the build carries into the docs:

- **openclaw** is an orchestrator, the same layer as AgentMux itself: a harness that drives other agents. It is preview so that it does not read as a peer of the lab harnesses.
- **dsh** is a DeepSeek preview.
- **aider** and **crush** are terminal-tier (their own UI in a terminal pane, no ACP), which already makes them second-class in the pane.
- **muxcode** is AgentMux's own fork-based harness; keeping it preview is a product decision, not a quality one (D4).

## 7. The flat-rate versus per-token nuance (recorded)

A lab's flat-rate subscription usually applies only through the lab's own harness. A third-party harness signed in to the same subscription can be billed per token as "extra usage" instead of from the flat allowance. The one confirmation found so far is Pi's own documentation (relayed by QTest's research; not re-read here). **Treat it as true for the doors, unverified for each lab, and re-check it per lab before any text says it.**

Consequences:

1. **It is the reason the subscription doors stay first-class**: each is the only harness where the subscription's flat rate is the expected bill. The picker's door cards can say "uses your plan" only for these.
2. **A harness signed in to someone's subscription is not "free"** even when no key is involved. The cost feature (#29360) must therefore show, per agent, which kind of billing applies: *plan* (the lab's own harness on its subscription), *key* (a model route or a BYO key, counted by the gateway in dollars, model-routes spec section 9) or *unknown* (a third-party harness on a subscription; per-token billing possible). The Free and Local tags (`provider-access.ts`) keep their meaning and must not be shown on a harness that bills extra usage.
3. **Model routes are always per token**, by construction, and the gateway already counts them; that is the one place the dollar figure is exact.

## 8. Existing agents and migration

- No migration of stored data. The tier is read at display and at image build time.
- An agent on a preview harness shows a small "Preview" tag where its harness shows (card, header icon), and nothing else changes.
- A template whose provider is preview stays in the picker under Preview; its agents are unaffected.
- Antigravity agents keep working and keep their managed install. The Gemini door does not adopt them: they are different harnesses with different sessions.

## 9. Decisions for the operator

| # | Decision | Recommendation |
|---|---|---|
| D1 | The keep list is claude, codex, gemini, copilot, grok, muse, qwen, kimi | Yes, as given |
| D2 | The demote list is aider, cline, crush, opencode, kilo, openclaw, dsh, muxcode | Yes, as given |
| D3 | Antigravity collapses into Gemini, with Gemini kept | Yes; flip only if Google's flat rate applies solely through Antigravity (unverified) |
| D4 | muxcode is preview | Yes for now; it can return to first-class when the product decides |
| D5 | Claude Code on a model route is an accepted long-term way to run non-Anthropic models | Yes while the general slot is open; revisit when #29364 resolves |
| D6 | The general slot stays named "TBD: goose or pi" until #29364 | Yes; the spec and the build do not wait |
| D7 | Preview means not preinstalled, below the fold in the picker, never a default; existing agents unaffected | Yes |
| D8 | The picker's preview section is collapsed by default and remembers its state | Yes |
| D9 | The cost feature (#29360) shows plan, key or unknown billing, as in section 7 | Yes; to be written into its spec |

## 10. Build plan (task #29363, after approval)

| PR | What | Effort (agent-days, an estimate, not measured) |
|---|---|---|
| 1 | The `tier` field in the registry and the catalog, set for all 19, the parity test and the three registry tests of section 5; the Preview tag in the toolchain pane | 1 |
| 2 | The picker: first-class cards, the collapsed Preview section, the remembered state; look checks on macOS and Linux | 1 to 2 (Agent1 and Camper) |
| 3 | The container image list reads the tier (with the model-routes P4); docs: the page per tier | 1 |

Every PR follows the usual checks (the change gates, a changeset, and a look check on macOS and Linux for the picker).

## 11. What I did not do

- I did not read the vendors' pricing terms; section 7 rests on one relayed documentation statement and is marked unverified per lab.
- I did not run any harness. The keep and demote lists are the operator's, taken as given.
- I did not decide goose or pi; I recorded what is known and why the slot can stay open.
