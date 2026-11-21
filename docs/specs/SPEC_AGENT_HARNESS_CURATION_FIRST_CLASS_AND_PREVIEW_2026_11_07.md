# Spec: curate the agent-pane harnesses: eight first-class doors, everything else preview

**Date:** 2026-11-07, revised 2026-11-08 (task #29362; the build is #29363)
**Status:** active — the recommended keep and demote lists (section 9) are approved. Step 1, the tier field and the Tools pane groups, is PR #7993; the picker's collapsed Preview section and the docs and billing wording remain.
**Revised 2026-11-20 (Wren@charlie, task #48203); signed off by the author (Agent5@narko) with the four flags answered below**, against `origin/main` at `61acd3b5` (v0.60.36). The decisions, the keep and demote lists and the intent are the author's as written. Only statements that main has made untrue are changed; each changed passage is marked **[REV n]** and listed in "Changes since the draft" below. A correction that touches a decision the operator is asked is flagged (F1 to F4), not decided.
**Author:** Agent5@narko
**Read from:** `origin/main` at `ad233144b` (v0.60.20); the auth classes in section 3 re-read from `identity/resolver/provider.rs` and `providers/*.rs` on 2026-11-08. Facts about other products are marked where they were not verified.
**Builds on:** `SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md` (the model route and the gateway), the provider registry (**[REV 1]** `crates/srv-core/src/backend/providers.rs`, `providers/*.rs`) and the frontend catalog (`frontend/app/view/agent/providers/catalog.ts`).
**Related:** #29360 (cost on the card and lists), #27459 P4 (container default and the list of harnesses without an image; **[REV 4]** merged as #6919 and #7204), #29414 (a bring-your-own harness through an ACP manifest), and the gap that a route's models are compiled into srv (section 12).

**Revision 2026-11-08, per the operator (relayed by AgentY):** (1) the "general harness" slot is dropped: every generic harness is preview, and the general path is Claude Code on a route, so there is no harness to crown (the goose-or-pi spike #29364 is cancelled); (2) the doors are split by how they sign in (section 3), because only the OAuth ones have a clean "uses your plan" story; (3) #29414 and the compiled-in model list are recorded as related work.

## Changes since the draft (2026-11-20)

Facts re-read from `origin/main` at `61acd3b5`. The PR is the squash commit that introduced the change.

| REV | Where | The draft said | Main says now | PR |
|---|---|---|---|---|
| 1 | Header, section 3, section 5, section 12 | `crates/srv/src/backend/providers.rs`, `providers/*.rs`, `identity/resolver/provider.rs`, `model_gateway/catalog.rs` | the same files under `crates/srv-core/src/...` (`catalog.ts` and `catalog-parity.test.ts` did not move) | #7439 (2026-11-15) |
| 2 | Section 1, section 3 (intro, grok row, muse row), D9, D10 | grok and muse have no `provider_class` arm; "five OAuth, three token"; wiring grok is a separate task | both are OAuth-class, signed in by their login file (`provider.rs`: claude, codex, openclaw, gemini, copilot, grok, muse). Six OAuth doors (claude, codex, gemini, copilot, grok, muse) and two key doors (kimi, qwen). D10 is done | #6843 (2026-11-08) |
| 3 | Section 2, Install | managed download for antigravity and goose | `cli_download.rs` holds goose, grok, muse and antigravity (hash-checked); `cli_uv.rs` holds aider and kimi | #6631, #6618 (2026-11-06) |
| 4 | Header "Related", section 5 table (Container images), section 10 PR 3 | a first-class harness "is in the image (the list the container default P4 reads)"; P4 is related future work | P4 is merged. The base image carries no provider CLI (it installs into a volume on first start); the gate is `has_container_image(provider)`, true for `claude` only, in `crates/srv/src/server/app_api/agent_spawn/container.rs`, mirrored by `containerSupported` in `frontend/app/view/agent/defaults/cli-catalog.ts`. Container installers for goose, grok, muse, antigravity, aider and kimi exist (`backend/container/cli_release.rs`) but the gate excludes them | #6919 (2026-11-10), #7204 (2026-11-13) |
| 5 | Section 2 (model route), section 7 item 3 | one OpenRouter route; the cost feature not started | a second gateway-only route carries an agent's own Anthropic API key; a route agent's cost is the provider's own reported charge; the spend chip and the daily caps show route and API-key agents (nothing for a login agent) | #7618 (2026-11-17), #6871 (2026-11-10), #6946 and later caps |
| 6 | Section 3 (Antigravity into Gemini) | "Not verified: whether Google sells any flat-rate access that applies only through Antigravity" | `frontend/app/view/agent/defaults/provider-access.ts` records that Gemini CLI's personal-account free tier ended 2026-06-18 and names Antigravity as its successor. A vendor fact as the repo states it, not verified here. **Flag F3** | before 11-05 |
| 7 | Section 8 | a "Preview" tag on the header icon | the header icon now shows the harness with the answering model as a badge, so where the tag goes is a build detail to re-place | release 0.60.22 |

### The four flags, answered by the author (2026-11-20)

- **F1 (REV 4): `tier` does not feed the container gate.** The container gate stays `has_container_image` (claude only today) with its frontend mirror `containerSupported`; a container installs its harness on first start, as main does. `tier` decides the picker, the defaults and the docs, and nothing about containers. So the first-class set is not "the set the container default can cover": a first-class harness other than claude runs native until its container support is added (the installers in `backend/container/cli_release.rs` exist), and the picker's existing "needs the host" marker says so. Widening the gate for goose, grok, muse, antigravity, aider or kimi is a separate decision, not this spec's.
- **F2 (REV 2): muse's card does not say "uses your plan" until its login is checked** against a real sign-in. The five OAuth doors whose login is a lab account (claude, codex, gemini, copilot, grok) may say it, as section 7 says; muse says "signs in with your Meta account" and nothing about billing. If a check shows its login is a plan, the card changes then.
- **F3 (REV 6): D3 stays as written, with the new fact in front of the operator.** The recommendation (Gemini kept, Antigravity collapsed) is unchanged; the operator's D3 answer should be given knowing that Gemini CLI's personal-account free tier ended on 2026-06-18 and that the repo names Antigravity as its successor.
- **F4: kimi is a native-only door.** Its key is the person's own, in the harness's environment (`MOONSHOT_API_KEY`), as an `ApiKey` door has always been. It is not a container door (no installer gate for it), so no key goes into a container; the rule that a container agent sees no key is not touched. If kimi's key should go through the Vault and the gateway like a route's, that is a separate change.

Not checked in this revision: vendor billing terms (section 7), the effort estimates, the look checks, and the task numbers (#29360, #29363, #29364, #29414 leave no trace in the repo).

## 1. The ask, and the answer in one page

The operator's direction, relayed by AgentY on 2026-11-07: curate the harness set. Today there are 19 providers; keep a short first-class list, and keep the rest as an installable preview tier, not delete them.

**The answer.**

- **Eight first-class doors**, each the lab's own harness: `claude`, `codex`, `gemini`, `copilot`, `grok`, `muse`, `qwen`, `kimi`. Antigravity collapses into Gemini (section 3). **[REV 2]** They are not alike: six sign in with the lab's account (OAuth; grok and muse since #6843), two with a token or key (section 3), and only the first group can promise "uses your plan" (muse: flag F2).
- **No general harness.** Every bring-your-own-key and OpenRouter model runs in Claude Code on a route (the model-routes spec), with the full MuxAPI toolset and the key in the Vault. There is nothing to crown, so `goose` and `pi` are preview with the rest.
- **Eleven preview**: `aider`, `cline`, `crush`, `opencode`, `kilo`, `openclaw`, `dsh`, `muxcode`, `goose`, `pi`, and `antigravity` (collapsed). Still installable and still working for an agent that already runs on one; just not preinstalled, not first in the picker, not in the defaults.
- **The mechanism is one field**, `tier` (`first_class` or `preview`), on the provider in the registry and in the catalog, read by the picker, the defaults and the docs (section 5); it does not touch the container gate. Nothing is removed from the code.
- **Why the auth class decides the story** (section 7): a lab's subscription usually buys flat-rate access only through the lab's own harness, and only an OAuth sign-in makes that a property of the account. A token or key bills what the token or key bills.

## 2. What exists today (read from `origin/main`)

- **19 providers** in the registry, mirrored in `catalog.ts`: claude, codex, muxcode, gemini, qwen, openclaw, kimi, copilot, pi, antigravity, kilo, cline, opencode, goose, grok, dsh, aider, crush, muse. A provider is a `ProviderConfig` (`providers.rs`): launch args, resume flag, auth isolation, pinned npm package or managed install, supported vendors, the startup instructions file.
- **No tier field.** Every provider is equal in the code. The "New agent" panel offers them all; some carry a Free or Local tag (`provider-access.ts`, `provider-access-model.ts`).
- **Install is already per harness**: npm packages with a pinned version, or a managed download **[REV 3]** (goose, grok, muse and antigravity, pinned and hash-checked, #6631; aider and kimi through a pinned uv, #6618), and a one-click install for the vendor-installed CLIs (task #138). So "installable, not preinstalled" needs no new installer.
- **Hide and unhide of seeded picker templates exists** (`Agents hide`, `unhide`), and `reseedagents`. Demotion must not rely on it: hiding is a per-person choice about the picker, not a property of the harness.
- **Account binding differs.** Gemini binds per account (`GEMINI_CLI_HOME`); Antigravity's `agy` signs in once per machine and reads no per-account directory, so there is no account to bind (comment in `catalog.ts`). Account switching and failover (`agent.account.switch`) therefore work for Gemini and not for Antigravity.
- **The model route** (the model-routes spec) lets Claude Code run on OpenRouter models through srv's gateway, billed to a key in the Vault. **[REV 5]** Since 2026-11-17 a second route carries an agent's own Anthropic API key (#7618). A route agent's cost is the provider's own reported charge (#6871), and the spend chip and daily caps show route and API-key agents only (none for a login agent).

## 3. The doors, split by how they sign in

Read from `provider_class()` (**[REV 1]** `crates/srv-core/src/identity/resolver/provider.rs`) and the provider files on 2026-11-08, **[REV 2]** re-read on 2026-11-20. `provider_class` is the gate for spawn-time identity and the per-account config directory: `OAuth` for `claude`, `codex`, `openclaw`, `gemini`, `copilot`, **`grok` and `muse` (added by #6843, signed in by their login file)**; `ApiKey` for `kimi` (`MOONSHOT_API_KEY`); no arm for `qwen`.

**OAuth doors — the lab's account signs in, so "uses your plan" is true:**

| Harness | Door | Notes |
|---|---|---|
| claude | Anthropic (Claude plan) | wired in `provider_class`; also the harness for model routes |
| codex | OpenAI (ChatGPT plan) | wired |
| gemini | Google (Gemini CLI; Google account) | wired; **the Google door** |
| copilot | GitHub (Copilot plan) | wired |
| grok | xAI (SuperGrok or X Premium+) | **[REV 2]** real OAuth through `grok login`, **wired into `provider_class` by #6843** (signed in by its login file `auth.json`; `GROK_HOME` config directory). The wire-up the draft flagged is done. |
| muse | Meta (Meta login) | **[REV 2]** moved here from the token table: OAuth-class in `provider_class` since #6843 (signed in by `muse/auth.json`, `XDG_CONFIG_HOME`). **Flag F2:** whether the login is a plan or a token was not verified (the draft's own note), so whether its card may say "uses your plan" is open. |

**Token or key doors — a token or key is billed for what it is, with no plan promise:**

| Harness | Door | Notes |
|---|---|---|
| kimi | Moonshot (`MOONSHOT_API_KEY`) | `ApiKey` class; no native startup-instructions file today |
| qwen | Alibaba | its OAuth free tier was retired on 2026-04-15; the harness stays for a key and the open models; no `provider_class` arm |
| muse | (moved) | **[REV 2]** moved to the OAuth table above: it is OAuth-class since #6843 |
| GLM | Z.ai (a token) | **no GLM harness exists in the registry.** GLM models run today in Claude Code on a route. A GLM door would need its own harness; recorded, not proposed |

**Antigravity into Gemini, which one stays: Gemini (recommendation).** Both are Google doors. Gemini CLI binds per account (so account switch and failover work), is on npm with a pin, and takes the Google account's access. Antigravity needs no per-account directory (nothing to bind or fail over), is a managed native download, and is the newer product. Keeping Gemini keeps the account features and the simpler install. **Not verified:** whether Google sells any flat-rate access that applies only through Antigravity; if it does, the operator may prefer the opposite, and the same mechanism supports it (flip which one carries `first_class`). **[REV 6, flag F3]** In-repo evidence now bears on this: `frontend/app/view/agent/defaults/provider-access.ts` records that Gemini CLI's personal-account free tier ended 2026-06-18 and names Antigravity as its successor. That is a vendor fact taken from the repo, not verified here; the recommendation above is left as the author wrote it. Antigravity's existing agents keep working as preview (section 8).

## 4. No general harness: the general path is a route

The earlier draft left one slot open for a general harness (`goose` or `pi`). It is dropped. Claude Code on a route already runs any OpenRouter model, on the full MuxAPI toolset, with the key in the Vault and never in the agent (the model-routes spec). `goose` and `pi` become preview with the other generic harnesses (`aider`, `cline`, `crush`, `opencode`, `kilo`, `dsh`, `muxcode`).

Facts kept for whoever reopens it, none of which this spec relies on:

- `pi` had no MuxAPI tools at the shipped pin (`pi-acp` 0.0.34); `goose` has no sign-in of its own (`goose configure` picks a provider and key).
- A route serves the Anthropic wire protocol only. Models and services on other protocols are not reachable that way. That gap is what #29414 (bring your own harness through an ACP manifest) is for, not a reason to crown a harness now.

## 5. The mechanism

One new field on the provider, written in the registry and the catalog (and kept equal by the existing parity test, `catalog-parity.test.ts`):

- `tier`: `first_class` or `preview`.
- **[REV 1]** `crates/srv-core/src/backend/providers.rs`: `ProviderConfig.tier`. Every `providers/*.rs` sets it. A test fails if more than 10 are `first_class` or if a provider has none.
- `frontend/app/view/agent/providers/catalog.ts`: the same value, checked against the registry by the parity test.

**What the tier changes** (and nothing else):

| Where | `first_class` | `preview` |
|---|---|---|
| New agent picker | cards at the top, in a fixed order | under a collapsed "More agents (preview)" section, each card tagged Preview |
| Container images | **[REV 4, answered F1]** no change: the base image carries no provider CLI, and the gate is `has_container_image` (`crates/srv/src/server/app_api/agent_spawn/container.rs`, claude only), mirrored by `containerSupported` in `frontend/app/view/agent/defaults/cli-catalog.ts`. The tier does not read or change either list | no change, as for first-class |
| Defaults and templates | may be a template's default; may be the "harness without an image" fallback | never a default |
| `Agents spawn` and `create` | as today | as today, with the harness named; the answer says "preview" |
| Toolchain pane | listed first | listed under Preview |
| Docs | the harness pages are written and kept current | one short page per harness: installable, not supported to the same depth |

**What it does not change:** an agent already running a preview harness keeps running, resumes, and keeps its history and memory; no stored record changes; the harness code, its translator and its tests stay. Preview means "not promised", not "removed".

**The picker.** The top section is the first-class cards; the second is collapsed by default and remembers its state per person. A search or the Free and Local filters search both. The exact component work (cards, the tag, the collapsed section, zoom, the look checks on macOS and Linux) is for Agent1 and Camper, with AgentX on the product direction; the build task names them.

## 6. The demote list

`aider`, `cline`, `crush`, `opencode`, `kilo`, `openclaw`, `dsh`, `muxcode`, `goose`, `pi`, and `antigravity` (collapsed into gemini). Notes the build carries into the docs:

- **openclaw** is an orchestrator, the same layer as AgentMux itself: a harness that drives other agents. It is preview so that it does not read as a peer of the lab harnesses. (It is OAuth-class in `provider_class`; its tier does not change that.)
- **dsh** is a DeepSeek preview.
- **aider** and **crush** are terminal-tier (their own UI in a terminal pane, no ACP), which already makes them second-class in the pane.
- **muxcode** is AgentMux's own fork-based harness; keeping it preview is a product decision, not a quality one (D4).
- **goose** and **pi** are the generic harnesses that were candidates for the general slot; a route covers that need (section 4).

## 7. The flat-rate versus per-token nuance (recorded)

A lab's flat-rate subscription usually applies only through the lab's own harness. A third-party harness signed in to the same subscription can be billed per token as "extra usage" instead of from the flat allowance. The one confirmation found so far is Pi's own documentation (relayed by QTest's research; not re-read here). **Treat it as true for the OAuth doors, unverified for each lab, and re-check it per lab before any text says it.**

Consequences:

1. **Only the OAuth doors carry a plan story**: each is the only harness where the account's flat rate is the expected bill, so the picker's door cards can say "uses your plan" for claude, codex, gemini and copilot (and grok once it is wired). A token or key door (kimi, qwen, muse, a GLM token) is billed for what the token or key is, and its card must not say "uses your plan".
2. **A harness signed in to someone's subscription is not "free"** even when no key is involved. The cost feature (#29360) must therefore show, per agent, which kind of billing applies: *plan* (an OAuth door on its own harness), *key* (a model route, a BYO key or a token door, counted by the gateway in dollars where it is a route, model-routes spec section 9) or *unknown* (a third-party harness on a subscription; per-token billing possible). The Free and Local tags (`provider-access.ts`) keep their meaning and must not be shown on a harness that bills extra usage.
3. **Model routes are always per token**, by construction, and the gateway already counts them; that is the one place the dollar figure is exact. **[REV 5]** Since #6871 the figure for a route agent is the provider's own reported charge, and the spend chip with its daily caps already shows route and API-key agents; nothing shows *plan* or *unknown* for a login agent yet, so that part of #29360 is still to build.

## 8. Existing agents and migration

- No migration of stored data. The tier is read at display and at image build time.
- An agent on a preview harness shows a small "Preview" tag where its harness shows (card, header icon), and nothing else changes. **[REV 7]** Since release 0.60.22 the header icon shows the harness with the answering model as a badge, so where the tag goes is a build detail to re-place.
- A template whose provider is preview stays in the picker under Preview; its agents are unaffected.
- Antigravity agents keep working and keep their managed install. The Gemini door does not adopt them: they are different harnesses with different sessions.

## 9. Decisions for the operator

| # | Decision | Recommendation |
|---|---|---|
| D1 | The keep list is claude, codex, gemini, copilot, grok, muse, qwen, kimi | Yes, as given |
| D2 | The demote list is aider, cline, crush, opencode, kilo, openclaw, dsh, muxcode, goose, pi | Yes, as given |
| D3 | Antigravity collapses into Gemini, with Gemini kept | Yes; flip only if Google's flat rate applies solely through Antigravity (unverified). **[REV 6, answered F3]** the repo records that Gemini CLI's personal free tier ended 2026-06-18 and names Antigravity as its successor: decide with that in front of you |
| D4 | muxcode is preview | Yes for now; it can return to first-class when the product decides |
| D5 | Claude Code on a route is the general path for non-lab models; no general harness is crowned | Yes (this is the operator's direction of 2026-11-08) |
| D7 | Preview means not preinstalled, below the fold in the picker, never a default; existing agents unaffected | Yes |
| D8 | The picker's preview section is collapsed by default and remembers its state | Yes |
| D9 | The cost feature (#29360) shows plan, key or unknown billing, and only the OAuth doors say "uses your plan" (section 7) | Yes; to be written into its spec. **[REV 2, answered F2]** muse is OAuth-class since #6843, but its login is unverified as a plan, so its card does not say "uses your plan" until it is checked |
| D10 | **[REV 2]** Done: `grok` (and `muse`) are wired into `provider_class` as OAuth doors | Settled by #6843; no decision left |

(D6, "the general slot stays TBD", is withdrawn with the slot.)

## 10. Build plan (task #29363, after approval)

| PR | What | Effort (agent-days, an estimate, not measured) |
|---|---|---|
| 1 | The `tier` field in the registry and the catalog, set for all 19, the parity test and the registry tests of section 5; the Preview tag in the toolchain pane | 1 |
| 2 | The picker: first-class cards, the collapsed Preview section, the remembered state; look checks on macOS and Linux | 1 to 2 (Agent1 and Camper) |
| 3 | **[REV 4, answered F1]** Docs: the page per tier, and the door cards' billing wording by auth class (muse without a plan claim, F2). The container gate is not changed by this spec | 1 |

Every PR follows the usual checks (the change gates, a changeset, and a look check on macOS and Linux for the picker).

## 11. What I did not do

- I did not read the vendors' pricing terms; section 7 rests on one relayed documentation statement and is marked unverified per lab.
- I did not run any harness or any sign-in. The auth classes in section 3 come from the registry and `provider_class()`; the `grok login` OAuth, the Qwen retirement date, Muse's login and GLM's token are as relayed by AgentY on 2026-11-08, not verified here.
- The keep and demote lists are the operator's, taken as given.

## 12. Related future work

- **#29414, a bring-your-own harness through an ACP manifest:** the answer for models and services a route cannot serve (other wire protocols), and for a harness this list does not ship. It makes the preview tier smaller over time.
- **A route's models are compiled into srv** (**[REV 1]** `crates/srv-core/src/backend/model_gateway/catalog.rs`): a new OpenRouter model needs a release before any instance lists it, as the GLM, Qwen and Kimi additions of 2026-11-08 showed. The model-routes spec's earlier idea, to read the route's own models endpoint and cache it, is the fix; it is not in this spec's scope.
