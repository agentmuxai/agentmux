# Spec: Connectors show every way to connect each provider, with plain explanations

**Status:** draft for the operator's OK (task #27097; the build is #27098)
**Date:** 2026-11-14
**Author:** Naki-2@narko
**Verified against:** `agentmux` `main` @ `086e28e98` (code read) and the vendor pages named in §6, fetched 2026-11-14.
**Builds on:** `SPEC_CODEX_OAUTH_IN_CONNECTORS_AND_EMAIL_LABELS_2026_10_22.md` §6 (Codex sign-in in Connectors, never built), `SPEC_CONNECTORS_CARDS_2026_10_13.md` (the cards).

## 1. Ask

The operator, 2026-11-06: "we already have an oauth version of openai, but in the connectors it appears as only the key. lets also add explations about them, including the names of the subscriptions and the keys and other pointers".

## 2. What is wrong today (read in code)

1. **OpenAI is key-only.** `SERVICE_CATALOG` gives the OpenAI tile `authModes: ["key"]` and the blurb "API key" (`accounts/accounts-catalog.ts`). Only Anthropic offers a sign-in.
2. **The sign-in already exists underneath.**
   - `provider_class("codex")` is OAuth-class (`CODEX_HOME`), and the probe reads `auth.json` (#5323).
   - A Codex pane's own sign-in makes an account `codex-oauth` (`identity_auth_persist.rs`).
   - `brandForProvider` already files it under the OpenAI tile (`provider-brand.ts`).
   - What is missing is a way to add one, or reconnect it, from Connectors. That was Part C of the 10-22 spec.
3. **The sign-in panel is Claude-named, but its mechanics are not.** `ClaudeLoginPanel` and its state, session and view files use `CLAUDE_PROVIDER` (`claude-login-model.ts`). Everything they do (`runProviderLogin`, the account directory, the URL capture, the terminal fallback) takes a provider.
4. **The explanations are one generic pair of lines** for every tile (`modeLabel`): "Connect with OAuth: Browser login, no key to manage" and "Add API key / token: Validated & stored in your OS keychain". They name no plan, no key, no page.
5. **Most providers have no tile at all.** Connectors has eight tiles (AgentMux, GitHub, Google, AWS, OpenAI, Anthropic, Slack, Custom). The New agent panel offers twenty-odd agent CLIs, and some of them have a sign-in AgentMux already runs:

| Agent CLI | AgentMux treats its credential as | Tile today |
|---|---|---|
| Claude Code | sign-in (OAuth class), key (`ANTHROPIC_API_KEY`) | Anthropic |
| Codex CLI | sign-in (OAuth class), key (`OPENAI_API_KEY`) | OpenAI (key only) |
| Gemini CLI | sign-in (OAuth class); `provider-brand` files it under Google | none for the CLI sign-in; the Google tile is the Gmail/Calendar/Drive connectors |
| GitHub Copilot CLI | sign-in (OAuth class); filed under GitHub | none for the CLI sign-in |
| Grok Build | sign-in (OAuth class) | none |
| Muse Code | sign-in (OAuth class) | none |
| OpenClaw | sign-in (OAuth class) | none |
| Kimi Code CLI | key (`kimi` is key class, injected as `MOONSHOT_API_KEY`) | none |
| Mux Code, Qwen Code | `authType: "api-key"`, no account class | none |
| Antigravity, Pi, Kilo, Cline, OpenCode, goose, Aider, Crush, DeepSeek Harness | `cli-managed`: the CLI asks for its own credentials inside the pane | none |

6. **Where a key lives.** An account's key is stored by srv's secret store: the OS keychain, or a `0600` file store on a machine with no keychain (`identity/secret_store.rs`). It is not the Vault. The task text says "the Vault"; the copy follows the code (see §9, question 1).

## 3. What "every way to connect" means

**An option is shown only if AgentMux supports it today.** The explanation never advertises a path the app can't do. Three kinds:

- **Sign in** (a subscription): AgentMux runs the CLI's own login and keeps the account's directory. Shown with a Connect button.
- **API key**: an account of kind `api_key`, stored as in §2.6. Shown with the key form.
- **Signs in inside the agent**: the CLI asks for its own credentials the first time it runs in a pane. Shown as text and a pointer, with no button, because there is nothing for Connectors to hold.

Per provider, after this build:

| Provider (tile) | Sign in | API key | Inside the agent |
|---|---|---|---|
| Anthropic | Claude plan (exists) | Claude Console key (exists) | |
| OpenAI | **ChatGPT account (new)** | OpenAI Platform key (exists) | |
| Google | Gmail, Calendar, Drive (exists) | | Antigravity (Google plan), Gemini CLI |
| GitHub | GitHub App (exists), Copilot (PR 2) | personal access token (exists) | |
| xAI (new tile, PR 2) | Grok Build sign-in | | |
| Moonshot (new tile, PR 2) | | Moonshot Platform key | Kimi Code sign-in |
| Other agents (a list under the tiles) | | | Muse Code, Qwen Code, DeepSeek Harness, OpenRouter-routed agents, Pi, Kilo, Cline, OpenCode, goose, Aider, Crush |

AWS, Slack, Custom and AgentMux keep their tiles and get one short explanation each (§4.4).

## 4. Design

### 4.1 One data file

`frontend/app/view/accounts/provider-guide.ts` is the only place a plan name, key name, prefix, link or price appears. Components read it and write none of it.

```ts
/** One checked fact: where it came from and the day it was read there. */
interface Fact<T> { value: T; source: string; checked: string }   // checked = "2026-11-14"

interface ConnectOption {
    kind: "signin" | "key" | "inside";
    title: string;                       // "Sign in with ChatGPT", "API key"
    what: Fact<string>;                  // one plain sentence
    plans?: Fact<string[]>;              // names exactly as the vendor writes them
    billedAgainst: Fact<string>;         // "your ChatGPT plan's Codex usage", "per token, at API prices"
    drives: string[];                    // agent CLIs it signs in
    key?: { calledOnVendorSite: Fact<string>; prefix?: Fact<string>; createAt: Fact<string>; env: string };
    pickWhen: string;                    // "Have ChatGPT Plus? Sign in."
}
interface ProviderGuide { brand: AccountProvider; options: ConnectOption[]; pointers: { pricing?: Fact<string>; usage?: Fact<string>; limits?: Fact<string> } }
```

- **A fact with no source is not shown.** The renderer drops it, and a test fails when a shown string has none (§7). So an unconfirmed plan name or prefix can never reach the screen.
- **Plan names stay in `plans`.** Renaming a plan is a one-line change in one file, with its new source and date.
- Where the vendor's own page doesn't name something, the entry leaves it out (§6.2 lists what was left out on 2026-11-14).

### 4.2 What the person sees

Under a tile, the chooser lists the options as today, but each is a card with its own words:

```
Connect OpenAI
 ┌ Sign in with ChatGPT ──────────────────────────┐   ┌ API key ───────────────────────────┐
 │ Use the ChatGPT account you already pay for.   │   │ A key from the OpenAI Platform.    │
 │ Codex runs on your plan, with no per-token bill│   │ Billed per token on your API account│
 │ ▸ Which plans, what it costs, when to pick it  │   │ ▸ Where to get one, what it costs  │
 └─────────────────────────────────────────────────┘   └─────────────────────────────────────┘
```

- The first two lines are always visible. The "▸" opens the rest in place: the plans or the key's name and prefix, the exact page to open (a link), what it is billed against, and "pick it when".
- Under the cards, one line of **pointers**: Pricing, Usage, Limits, as links, where the vendor has them.
- Tiles with one option (AWS, Slack, Custom) show the explanation directly.
- A connected account's card says which way it connected ("ChatGPT sign-in", "API key"), not only "OAuth (browser login)".

### 4.3 Sign in, for every provider

- `ClaudeLoginPanel` and its files become `ProviderLoginPanel` and take the provider; Claude keeps its behaviour and its tests. The names `claude-login-*` become `provider-login-*`.
- The panel takes `provider` and `brand` (what it calls the sign-in on screen: "Anthropic", "ChatGPT"); callers that give neither get Claude and Anthropic, so the agent pane's and the Stash's uses are unchanged.
- The OpenAI tile's sign-in opens it for Codex: the login URL is shown to open, with the terminal fallback the panel already has.
- The account is created unbound from Connectors, as for Claude.
- **Follow-up (PR 2), because each needs a real ChatGPT sign-in to check and the build machine has none:** the device code (`codex login --device-auth`) offered when the URL capture times out (10-22 spec §6.2), and Reconnect for a CLI sign-in's account, reusing its directory so its bindings survive. Today an account's Reconnect exists only for service sign-ins (`canReconnect`).
- Each later provider (Copilot, Grok) is a data entry plus a hand check of a real sign-in (§8); none is enabled unseen.

### 4.4 The other tiles

- **AgentMux, AWS, Slack, Custom:** keep their present two lines. They name no vendor plan or key prefix, so the data file has nothing sourced to add for them; an entry is added when there is.
- **GitHub:** keeps its two choices (`github-account-copy.ts`), now described by the same data (the App for repos and PRs; a personal access token). Copilot is added in PR 2 (§3).
- **Google:** the explanation says plainly that this tile connects Gmail, Calendar and Drive, and that Google's coding agents sign in inside the agent (Antigravity, Gemini CLI), with the line from §5.

### 4.5 Copy rules

- Current offerings only, no history (the alpha docs rule): no "was renamed", no "moved to", no "since".
- Plain words. A plan name is written as the vendor writes it. A price appears only with its source and billing period, or not at all.
- No promise AgentMux can't keep: nothing says "unlimited", and nothing names a plan the vendor's page doesn't tie to the agent.

## 5. The words (all of §5 comes from §6's sources, read 2026-11-14)

**Anthropic**
- *Sign in with your Claude plan.* "Use your Claude account. Claude Code runs on your plan's usage limits, with no per-token bill." Plans: Claude Pro, Max and Ultra. Team and Enterprise seats also include Claude Code (their sign-in is in Anthropic's help article, linked). Drives: Claude Code. Pick it when you have a Claude plan.
- *API key.* "A key from the Claude Console (platform.claude.com/settings/keys), starting `sk-ant-api`. Billed per token on your Console account, separate from any Claude plan. If a key is set, Claude Code uses it instead of your plan." Pick it when you're building something, running automation, or want per-token control.
- Pointers: Pricing (claude.com/pricing), Limits (platform.claude.com/usage/limits), Spend limits (platform.claude.com/settings/spend-limits).

**OpenAI**
- *Sign in with ChatGPT.* "Use your ChatGPT account. Codex runs on your plan's included Codex usage." Plans that include Codex: Free, Go, Plus, Pro, Business, Edu and Enterprise (the pricing page marks Free and Go as limited). Sign-in is needed for Codex cloud. Drives: Codex CLI. Pick it when you have ChatGPT.
- *API key.* "A key from the OpenAI Platform (platform.openai.com/api-keys). Codex then uses standard API pricing, not your ChatGPT plan, and has no cloud features (no cloud chats, PR reviews or Slack/Linear)." Billing: platform.openai.com/account/billing/overview. Pick it when you're building an app or have no ChatGPT plan.
- Pointers: Pricing (chatgpt.com/pricing), Codex plans (learn.chatgpt.com/codex/pricing).

**Google**
- Tile: "Connects Gmail, Calendar and Drive to your agents."
- Inside the agent: "Antigravity signs in with the Google account that holds your Google AI plan (Free, Pro or Ultra); there is also a Gemini API key from Google AI Studio (aistudio.google.com/apikey). Gemini CLI is for organisations with Gemini Code Assist Standard or Enterprise licences. Open an agent of that kind and it asks you."
  - Antigravity sign-in types are from Google's page. The README line on who Gemini CLI is for is Google's own text.

**GitHub Copilot** (PR 2)
- *Sign in.* "Use your GitHub account. Copilot plans: Free, Student, Pro, Pro+, Max, Business and Enterprise; every plan includes Copilot CLI. Usage is counted in GitHub AI Credits." Sign-in opens a browser, or shows a code on a remote machine.

**xAI** (new tile, PR 2)
- *Sign in.* "Use your Grok account. Grok Build signs in through your browser (a code for remote machines)." The plan names are not shown (§6.2). Pointer: x.ai.
- *Key:* console.x.ai, billed from prepaid credits. This is not an AgentMux option today (§3), so it is not offered.

**Moonshot** (new tile, PR 2)
- *API key.* "A key from the Kimi API Platform (platform.kimi.ai/console/api-keys), billed separately from a Kimi membership."
- Inside the agent: "Kimi Code has its own sign-in; open a Kimi agent and run /login."

**Other agents:** one line each: "Asks you to sign in or paste a key the first time it runs", with its docs link (from `catalog.ts`), and for OpenRouter-routed agents a key from openrouter.ai/keys (starts `sk-or-v1-`, paid from credits, each key can have a credit limit); for DeepSeek a key from platform.deepseek.com/api_keys, billed per token from a topped-up or granted balance.

## 6. Sources, and what was left out

### 6.1 Read on 2026-11-14

| Fact | Source |
|---|---|
| Claude Pro, Max, Ultra sign in to Claude Code; limits per plan; a set `ANTHROPIC_API_KEY` wins over the plan | support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan |
| Claude plans and that all paid ones include Claude Code | claude.com/pricing |
| Console key page, `sk-ant-api...`, key types, separate billing | platform.claude.com/docs/en/manage-claude/authentication; platform.claude.com/docs/en/api/overview |
| Codex: ChatGPT sign-in vs API key, device code, API pricing, cloud needs ChatGPT | learn.chatgpt.com/docs/auth (developers.openai.com/codex/auth redirects there) |
| Plans that include Codex; API-key column | learn.chatgpt.com/codex/pricing |
| ChatGPT plan names | chatgpt.com/pricing |
| OpenAI key page and billing overview | developers.openai.com/api/docs/quickstart |
| Copilot plans, CLI in all plans, AI Credits | docs.github.com/en/copilot/get-started/plans |
| Copilot CLI sign-in, device code, token variables | docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli |
| Gemini CLI for Code Assist organisations; Pro/Ultra on Antigravity; sign-in types | github.com/google-gemini/gemini-cli; geminicli.com/docs/get-started/authentication/ |
| Antigravity sign-in types and supported accounts | antigravity.google/docs/install/ |
| Grok Build sign-in, device code, `XAI_API_KEY`, console.x.ai | github.com/xai-org/grok-build (`02-authentication.md`) |
| xAI API prepaid credits | docs.x.ai/docs/key-information/billing |
| Kimi Platform key page; Kimi Code `/login` | platform.kimi.ai/docs/overview; moonshotai.github.io/kimi-code/en/configuration/providers |
| OpenRouter key page, `sk-or-v1-`, credits and limits | openrouter.ai/docs/api-reference/authentication |
| DeepSeek key page, per-token billing from balance | api-docs.deepseek.com; api-docs.deepseek.com/quick_start/pricing |
| Muse Code: browser sign-in or API key, `/login`, `META_API_KEY` | dev.meta.ai/docs/muse-code |

### 6.2 Left out because no vendor page of its own says it

These stay out of the copy until a vendor page does. They are on the build's list to re-check, not guessed.
- The OpenAI key prefix (the quickstart doesn't state one).
- xAI plan names and the `xai-` prefix (only third-party pages; xAI's own pages read didn't name them).
- The Moonshot key prefix and Kimi membership plan names.
- The DeepSeek key prefix.
- The Gemini API key prefix, and Google AI plan prices (a third-party page lists them; Google's own page read doesn't).
- Qwen Code's current sign-in and plans (the only first-hand page found is a repository doc; the rest is third-party).
- Muse Code plans and billing (its subscriptions page wasn't readable).
- Any ChatGPT, Claude or Copilot dollar price. The copy doesn't quote prices, since the pages move; it links the pricing page.

Two vendor pages disagree today: Gemini CLI's README says Google AI Pro and Ultra moved to Antigravity, while its authentication page still lists them under "Sign in with Google". The copy follows the README (§5) and the build re-reads both before it ships.

## 7. Tests

- **Data:** every `Fact` has an `https` source and a `checked` date (`YYYY-MM-DD`); every `plans` entry is non-empty; the renderer shows no string that lacks a source.
- **No history in the copy:** a test scans the guide's strings for "was renamed", "moved to", "formerly", "since 0." and "previously".
- **Catalog:** every option of kind `signin` or `key` maps to a provider AgentMux supports (`provider_class` is `Some`), so the guide can't offer what the app can't do. A mirror of the existing `oauth_class_matches_frontend_authtype_oauth_set` pattern.
- **Tiles:** OpenAI offers Sign in and API key; each tile's chooser shows its options' words from the data file; the "▸" opens the rest.
- **Sign-in:** `ProviderLoginPanel` for Claude behaves as before (the existing tests move with it); for Codex it starts `codex login`, shows the URL, offers the device code after the timeout, and creates an unbound `codex-oauth` account that groups under OpenAI.
- **By hand, in a build:** sign in to ChatGPT from Connectors (browser and device code); the account shows under OpenAI as "ChatGPT sign-in" and reads valid after a first Codex turn; reconnect keeps its bindings.

## 8. Plan

1. **PR 1:**
   - `provider-guide.ts` and its tests;
   - the explanation and pointers on the Anthropic, OpenAI and Google tiles (the others keep their lines, §4.4);
   - `ProviderLoginPanel` (renamed from the Claude panel, taking the provider), and the OpenAI sign-in with the terminal fallback;
   - this spec, committed with it.
2. **PR 2:** the device-code fallback and Reconnect for a CLI sign-in (§4.3), the xAI and Moonshot tiles, Copilot under GitHub, and the "Other agents" list, each sign-in enabled only after a real sign-in is seen working in a build (needs the operator's or a tester's account).
3. **Visual checks:** both PRs change how Connectors looks, so macOS (Clare, Masty, Maop) and Linux (Maricon, Opaz, Parlo) look checks run before each merge.

## 9. Open questions

1. **"Where AgentMux keeps it (the Vault)":** an account's API key is in the OS keychain (a file store on a machine with no keychain), not the Vault. The copy says "stored in your computer's keychain". Should Connectors keys move into the Vault (Agent2-6's lane), or is the keychain wording right for now?
2. **Kimi's key variable.** srv injects a Kimi key as `MOONSHOT_API_KEY` (`provider_class("kimi")`), while the Kimi Code docs list `KIMI_API_KEY` for the CLI and say it doesn't read shell variables unless `api_key_env` is declared. The build checks a bound Kimi account actually reaches the CLI before the Moonshot tile ships. If it doesn't, that is its own bug.
3. **Google individuals.** Should Connectors offer a Gemini CLI sign-in at all, now that Google's README sends individuals to Antigravity (which AgentMux treats as signing in inside the agent)? This spec says no, and shows text only.
4. **Staying current.** A plan list goes stale. Should a weekly Schedule job re-read the §6.1 pages and open a task when one differs, or is the dated source line enough for now?
