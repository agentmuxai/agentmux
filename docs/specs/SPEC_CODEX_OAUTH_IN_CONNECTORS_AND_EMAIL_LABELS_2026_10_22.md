# Spec: Codex sign-in in Connectors, and every account labelled by its email

**Status:** proposed
**Date:** 2026-10-22
**Author:** AgentA@Area54, at the operator's request
**Verified against:** `agentmux` `main` @ `cc46a8c21` (2026-10-22), by reading code, and live on 0.59.38 with a Codex agent (Coddy).
**Builds on:** `SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md` (email in `context.email`, `accountLabel`), `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` (Slice D, the Codex account UX, never started), `SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md` (the Claude sign-in this generalises), `SPEC_PROVIDER_LOGIN_THROUGH_SRV_2026_10_09.md` (the login backend).

## 1. Ask

The operator, 2026-10-22: "lets try to get Coddy working. he is a codex agent … also, write a spec to introduce Oauth to the connector's for codex, and we also want the email as the label for all providers, since that is always what each account, we can tell the provider from the icon and the account from the email. try to fix Coddy first, write the rest into a spec to file".

## 2. What was wrong with Coddy

Two separate faults, both affecting every Codex agent, not only Coddy.

### 2.1 Every model AgentMux offers for Codex is gone

Coddy's turns failed with `The 'gpt-5.5' model is not supported when using Codex with a ChatGPT account.`

- AgentMux hard-codes Codex's models: `gpt-5.5` (default), `gpt-5.4`, `gpt-5.1-codex-max`, `gpt-5.3-codex` (`frontend/app/view/agent/providers/catalog.ts`, the `codex` entry's `models`). srv repeats the default in `default_model_for` (`crates/srv/src/backend/providers.rs`).
- The account's own model list, which the Codex CLI fetches from OpenAI and caches in `$CODEX_HOME/models_cache.json`, now lists `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra` and `gpt-5.6-luna` (plus hidden internal entries). None of the four AgentMux offers is on it.
- `/model` only accepts values from the catalog (`commands/dispatch.ts`, enum args), so a user can't pick a working model from the pane.

**Done now, by hand:** Coddy's pane meta was set to `gpt-6-astra` (the same `agent:runtime` + `cmd:args` write `/model` makes) after one test turn on that model with Coddy's account. Coddy answered. A new Codex pane still starts on `gpt-5.5` and fails until §4 ships.

### 2.2 Every Codex account is marked "needs re-auth" at its first spawn

- The spawn-time probe (`crates/srv/src/identity/resolver/oauth_probe.rs`) reads `<dir>/.credentials.json` for `claude`, `codex` and `openclaw` alike.
- A ChatGPT sign-in through `codex login` writes `$CODEX_HOME/auth.json`, never `.credentials.json`.
- So the probe finds nothing and `inject.rs` persists `needs_reauth`. The srv log shows Coddy's account going `valid` → `needs_reauth` at 06:47:35, seconds after a working sign-in.
- The spawn is not refused, so the agent still runs, but the account then:
  - shows as needing reconnection in Connectors;
  - drops out of the Bind and "Switch to" pickers, which offer only `status === "valid"` accounts (`failure/bind-account-candidates.ts`).

## 3. Goals

1. A Codex agent starts on a model its account can use, and the model menu lists that account's models.
2. A working Codex sign-in reads as valid.
3. Connectors can sign in to Codex with a ChatGPT account, as it can for Claude, and can reconnect it.
4. Every account, of every provider, is labelled by its email wherever an account is named. The provider is told by its icon. A name is used only for an account that has no email (an API key, a custom bearer token).

## 4. Part A: Codex's models come from the account

- **Source.** The Codex CLI already keeps the account's list in `$CODEX_HOME/models_cache.json` (`fetched_at`, `client_version`, `models[]` with `slug`, `visibility`, `priority`). srv reads it from the bound account's directory through the existing `providers.models` RPC (today Claude only, `app-init.ts`). It returns the `visibility: "list"` entries in `priority` order.
- **Default.** The first listed entry is the default for a new pane. With no cache yet (a fresh sign-in before the first turn) a pane passes no `--model` and the CLI uses its own default. That beats a hard-coded id that may already be gone.
- **Catalog.** The curated list in `catalog.ts` and `default_model_for` become a fallback only, updated now to the current names so a pane opened before the cache exists isn't broken. `providers/runtime-defaults-consistency.test.ts` keeps the two copies equal.
- **Stored ids that disappear.** A pane whose stored model is no longer on the account's list is corrected to the list's default. The correction uses the existing `migration` path in `AgentRuntimeDropup.tsx`, which records the change without a restart. Today that path only maps within a family (`familyKey`), and `gpt-5.5` → `gpt-6-astra` crosses families, so it gains a "not offered any more" case.
- **`/model`** offers the account's list, and accepts any id the account lists.

## 5. Part B: the Codex status probe reads `auth.json`

- For `codex`, the probe reads `<dir>/auth.json`:
  - `tokens.refresh_token` present means `valid`;
  - an `OPENAI_API_KEY` there (an API-key sign-in) also means `valid`;
  - a missing file means `needs_reauth`.
- Expiry is left to the CLI, which refreshes the token itself, as the probe already does for Codex.
- `openclaw` is checked the same way against its own files before it keeps the `.credentials.json` rule.
- The host's credential baseline used to detect a finished login (`crates/cef/src/commands/cli_login.rs`) watches `auth.json` for Codex for the same reason.
- An account already stuck at `needs_reauth` heals on its next spawn, because the probe writes whatever it finds.

## 6. Part C: sign in to Codex from Connectors

### 6.1 Today

- The OpenAI tile in Connectors offers an API key only (`accounts/accounts-catalog.ts`).
- The one way to get a Codex ChatGPT account is to open a Codex agent, fail, and use the pane's sign-in (`runPaneLogin` → `runProviderLogin`).
- That path names the account `codex-oauth`.
- The Stash's Connect and Re-login buttons are Claude-only (`identity/agent-identity-links-panel.tsx`).

### 6.2 Design

- **The tile.** OpenAI gets the same chooser Anthropic has: "Sign in with ChatGPT" (OAuth) or "API key". OAuth opens a provider sign-in panel.
- **One sign-in panel.** `ClaudeLoginPanel.tsx` becomes a `ProviderLoginPanel` that takes the provider. Everything it does is already provider-driven:
  - `runProviderLogin` with `awaitTier1Completion`;
  - the account directory from `identity.ensureaccountdir`;
  - the URL capture (`match_codex_url` already exists in `identity/auth_patterns.rs`);
  - the terminal fallback.

  Claude keeps its current behaviour. This is `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` Slice D, §7.2.
- **How Codex signs in.**
  - `codex login` opens the browser and waits for its own `localhost` callback; the panel shows the URL to open, as it does for Claude.
  - `codex login --device-auth` gives a code to type at a URL. That is the fallback when the browser callback can't reach the CLI, for example on a remote host.
  - The panel offers the device code after the URL capture times out, instead of dropping straight to a terminal.
- **Reconnect.** The account card's Reconnect, and the Stash's Connect and Re-login, work for Codex. They reuse the account's directory, so its bindings survive.
- **Bind after sign-in.** As for Claude: from Connectors the account is created unbound. From an agent's failure row it is bound to that agent.
- **The App Server path** (`ChatgptDeviceCode`, `account/read` in `app_server_protocol.rs`, unwired) stays out of scope. It belongs to `SPEC_CODEX_APP_SERVER_FIRST_CLASS_PROVIDER_2026_09_12.md`.

## 7. Part D: every account labelled by its email

### 7.1 The rule

An account's label is its email. The brand icon beside it says which provider it is, so the label doesn't repeat it. An account without an email (API key, bearer token, AWS role) keeps the name the user gave it. Where two accounts would show the same label, each also shows its kind ("OAuth", "API key"). An example is the same email signed in to Claude twice: one through Connectors, one through a pane.

`accountLabel(a) = context.email || name` (`identity/identity-model.ts`) already says this. What is missing is an email for most providers, and some places that don't use `accountLabel`.

### 7.2 Where each provider's email comes from

All of these read files the CLI already wrote in the account's own directory. Nothing calls a provider API (the non-goal of the 09-23 spec stands).

| Provider | Source | Today |
|---|---|---|
| Claude | `<dir>/.claude.json` → `oauthAccount.emailAddress` | Done (`identity/account_email.rs`) |
| Codex (ChatGPT) | `<dir>/auth.json` → `tokens.id_token`, the JWT's `email` claim (read locally, signature not needed for a label) | None. A test pins `None` |
| Codex (API key) | none | Name |
| Gemini (Google sign-in) | `google_accounts.json` → `active` under the account's `GEMINI_CLI_HOME` (to be confirmed on a real sign-in) | None |
| Copilot | the GitHub login from the CLI's own config (a username, not an email) | None; shows the username |
| Google, GitHub, Slack connectors | ID token or user API at connect time (`oauth_client.rs`) | Done for Google |
| API keys, AWS, custom | none | Name |

- `email_from_oauth_dir` grows a case per provider.
- The existing backfill fills old accounts the next time accounts are listed. `refresh_account_email` writes only when the value differs, so a re-login as someone else updates the label.
- The ChatGPT plan (`chatgpt_plan_type` in the same token) can be a subtitle on the card. It's optional.

### 7.3 Places that name an account without `accountLabel`

Change these to use it:

- the Stash links table (`identity/agent-identity-links-panel.tsx`): `display_name`, else "Anthropic", else `name`;
- the agent Identity panel's account menu (`agent/components/AgentIdentityPanel.tsx`): `display_name || name`;
- the bind error toast (`agent/hooks/useAgentControllerStatus.ts`): `name`;
- the agent's startup context (`agent/startup/buildStartupPayload.ts`): `### codex — codex-oauth`. It becomes the provider and the email, so an agent knows which account it runs as;
- the `Identity` MuxAPI tool's `accounts` (`crates/srv/src/server/app_api/identity.rs`): add `email` and `label`.

### 7.4 Names

- New CLI sign-ins still get `name: "<provider>-oauth"`, but it is no longer shown anywhere an email exists.
- The account form lets the user give any account a display name (an "alias"). If set, it shows instead of the email. CLI sign-in accounts can't be edited at all today (the form hides Save for `kind: "oauth"`), so that changes too. An alias is optional. The default is still the email.

### 7.5 Privacy

As in the 09-23 spec §6:

- the email is already on the machine in the provider's own files;
- it is shown only locally;
- it is not logged at info or above;
- it goes nowhere new off the host.

Adding it to the startup context and the `Identity` tool gives it to the user's own agents on this machine, which already run with that account's credentials.

## 8. Tests

- **Models:**
  - `providers.models` for Codex reads a fixture `models_cache.json` and returns the listed models in priority order, without hidden ones;
  - no cache returns an empty list and the pane passes no `--model`;
  - a stored `gpt-5.5` is corrected to the list's default without a restart;
  - `/model gpt-6-astra` is accepted once the account lists it.
- **Probe:** for Codex, `auth.json` with a refresh token → `valid`, with an API key → `valid`, missing → `needs_reauth`. The current test that a missing directory is `needs_reauth` stays.
- **Email:**
  - `email_from_oauth_dir("codex", …)` reads the `email` claim from a fixture `auth.json` and returns `None` without one;
  - the backfill fills it for an existing account.
- **Labels:** each place in §7.3 renders the email for an account that has one and the name for one that doesn't, and two accounts with one email show their kinds.
- **By hand, in a build:**
  - sign in to Codex from Connectors, through the browser and through the device code;
  - the account card shows the email and stays valid after the first Codex turn;
  - bind it to a Codex agent from the agent's Bind menu;
  - the agent runs on a listed model.

## 9. Plan

1. **PR 1, fixes (§4, §5):**
   - the Codex model list from the account;
   - the fallback catalog updated;
   - the stored-id correction;
   - the probe reading `auth.json`.

   This is what keeps Codex agents working, so it goes first.
2. **PR 2, email labels (§7):**
   - the Codex and Gemini email sources;
   - every naming place through `accountLabel`;
   - the `Identity` tool fields;
   - the alias.
3. **PR 3, Codex sign-in in Connectors (§6):**
   - `ProviderLoginPanel`;
   - the OpenAI tile's OAuth mode;
   - device-code fallback;
   - Reconnect and the Stash buttons for Codex.

## 10. Open questions

1. When a Codex account's model list changes under a running pane, should the pane switch at its next turn, or keep running and show the drift notice it shows today?
2. Gemini's file layout under `GEMINI_CLI_HOME` needs one real sign-in to confirm before §7.2's Gemini row is built.
3. Should an alias, once set, also replace the email in the agent's startup context, or should the agent always see the email?
