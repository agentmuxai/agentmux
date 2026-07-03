# SPEC — API-sourced model catalog: keep the agent-pane model dropdown current

**Date:** 2026-07-02
**Type:** Implementation spec
**Status:** Ready to schedule
**Owner:** asaf
**Scope:** `agentmux-srv` (model-catalog fetch + cache + RPC) and `frontend/app/view/agent`
(registry overlay + dropdown convergence).

> **Problem.** The agent-pane model dropdown is a hand-curated list. Its `value`s are family aliases
> (`opus`/`sonnet`/`haiku`) that the Claude Code CLI resolves to *its current default*, but the **labels**
> ("Opus 4.8", "Sonnet 4.6", "Haiku 4.5") are compile-time constants that drift every time Anthropic ships
> a model or the CLI pin bumps. Today "Sonnet 4.6" is shown even though **Sonnet 5** (`claude-sonnet-5`) is
> current. We want a *robust*, non-hallucinated way to keep this list correct.
>
> **Answer.** Fetch the catalog from the **Anthropic Models API** (`GET /v1/models`) using the agent's
> existing OAuth token, cache it per CLI version, refresh on CLI install/upgrade, and have all dropdowns
> read it — with the curated list demoted to an offline fallback. Every dependency already exists in-tree.

---

## 1. Why not the alternatives

| Source | Verdict | Why |
|---|---|---|
| **Background Haiku prompt** ("what models does this CLI offer?") | ❌ Reject | Non-authoritative — bounded by the model's own knowledge cutoff and hallucination-prone. A wrong version string here becomes a bad `--model` value that fails at launch. The existing pane-header Haiku path (`app_api/session.rs:198-267`) is a per-turn one-shot with no cache — wrong tool *and* wrong cadence. |
| **Enumerate from the Claude Code CLI** | ❌ Not possible | The CLI exposes **no** list-models surface: `--model` only *accepts* aliases/IDs; `claude --help` has no `models`/`config` subcommand (confirmed `SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md:46`, `SPEC_CONTEXT_VISIBILITY_2026_06_17.md:134`). Only introspection is `claude --version` (`cli_handlers.rs:734-753`). |
| **Anthropic Models API `GET /v1/models`** | ✅ Primary | Authoritative, auto-tracks new models, returns real `id` / `display_name` / `max_input_tokens` / `capabilities`. Reachable with the agent's OAuth Bearer token — live-verified (§3). |
| **Curated static catalog, version-gated** | ✅ Fallback | The `providers/index.ts` list, kept as the bundled offline default for first-run / pre-auth / macOS-Keychain / network-down. |

**Design = Models API primary + bundled curated fallback.** This is the same shape
`SPEC_CONTEXT_VISIBILITY_2026_06_17 §5 P1` already reasoned through for context windows (same endpoint,
same catalog, same fallback) — share that infrastructure rather than build a parallel one.

## 2. Current state (what exists today)

- **TS registry** `frontend/app/view/agent/providers/index.ts`: `ProviderModel = { value, label, default?,
  description?, aliases? }` (`:36-42`); Claude's hardcoded list at `:196-200` (alias values, curated
  labels). Comment `:185-195` documents that labels are hand-synced on pin bumps — the drift source.
- **Rust `ProviderConfig`** `agentmux-srv/src/backend/providers.rs:33-83` carries `pinned_version` (`:77`;
  Claude `"2.1.198"` `:147`) but **no `models` field** — the catalog is TS-only today.
- **Three dropdown consumers, already drifting:** `/model` slash command reads the registry
  (`commands/global/runtime.ts:58-72`), but `AgentComposerStrip.tsx` and `AgentControlBar.tsx` **hardcode
  their own lists** (`SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md:39-42`). (Note: PR #1912 moves
  Mode/Model/Effort to the strip; the strip's Model select there still reads the registry, so this
  convergence work layers cleanly on top.)
- **CLI version detection:** `get_cli_version` (`cli_handlers.rs:734-753`) runs `claude --version`, 5s
  timeout. npm-latest via `toolchain.versions` (`cli_handlers.rs:635-675`).

## 3. The credential path exists (verified)

The whole design hinges on being able to call `GET /v1/models` authoritatively. Both halves are already
in-tree and verified:

1. **OAuth token extraction.** `cli_handlers.rs:344-369` reads the isolated
   `CLAUDE_CONFIG_DIR/.credentials.json` and parses `claudeAiOauth.accessToken` / `refreshToken`.
   (`identity/resolver.rs:131-143` reads the same for expiry.)
2. **Outbound Anthropic HTTP.** `identity/key_validator.rs:141-163` already does
   `GET https://api.anthropic.com/v1/models` via a shared `reqwest::Client` and parses `body.data[]` —
   today with an `x-api-key` header for API-key validation.
3. **OAuth reachability is verified — live-retested 2026-07-02.** Against a real **`subscriptionType:
   "max"`** credential (scopes `user:inference`, `user:sessions:claude_code` — a genuine Claude Max
   *subscription*, not an API key), `GET https://api.anthropic.com/v1/models` with
   `Authorization: Bearer <accessToken>` + `anthropic-version: 2023-06-01` and **no beta header** returned
   **HTTP 200** with 10 models. So a **subscription** OAuth token can read the catalog — this is a metadata
   read (no inference, no billing), which is why the subscription token is accepted even though it is
   **not** meant to call `/v1/messages` on the public API. (Consistent with
   `SPEC_CONTEXT_VISIBILITY_2026_06_17.md:136`.)

   Observed `data[]` (id | display_name), 2026-07-02:
   `claude-sonnet-5` | Claude Sonnet 5 · `claude-fable-5` | Claude Fable 5 · `claude-opus-4-8` | Claude
   Opus 4.8 · `claude-opus-4-7` | Claude Opus 4.7 · `claude-sonnet-4-6` | Claude Sonnet 4.6 ·
   `claude-opus-4-6` | Claude Opus 4.6 · `claude-opus-4-5-20251101` | Claude Opus 4.5 ·
   `claude-haiku-4-5-20251001` | Claude Haiku 4.5 · `claude-sonnet-4-5-20250929` | Claude Sonnet 4.5 ·
   `claude-opus-4-1-20250805` | Claude Opus 4.1. Confirms the current registry label "Sonnet 4.6" is stale
   (both `claude-sonnet-4-6` and `claude-sonnet-5` are live).

So the new fetch is: reuse #1 to get the token, reuse #2's pattern but send
`Authorization: Bearer <access_token>` + `anthropic-version: 2023-06-01` instead of `x-api-key`.

> **Note — subscription-token access is undocumented and metadata-only.** The `/v1/models` acceptance of a
> subscription OAuth token is not a documented API contract; treat it as best-effort (Anthropic could
> change it) and **only** call `/v1/models`, never `/v1/messages`, with this token. The §4 design already
> degrades gracefully (401 / failure → bundled fallback), so a future revocation only stops auto-updates,
> it doesn't break the dropdown.

**Known caveats (must handle):**
- **macOS Keychain.** On macOS the CLI stores creds in the Keychain, not `.credentials.json`
  (`cli_handlers.rs:371-378`) — there the file token is absent. → fall back to the bundled catalog.
- **Token expiry / 401.** OAuth access tokens expire (`SPEC_CONTEXT_VISIBILITY_2026_06_17.md:140`). →
  keep last-good cache, don't fetch synchronously against a possibly-stale token; refetch on next
  auth/install or on a 401.
- **Ceiling vs effective.** `max_input_tokens` is the model *ceiling*, not the plan's effective window
  (`:138`) — relevant only if we also consume context-window data here; out of scope for the label fix.

## 4. Design

### 4.1 Backend — `fetch_model_catalog`
Add (suggested `agentmux-srv/src/backend/model_catalog.rs`):
```rust
/// Fetch the live model catalog for a provider using the agent's OAuth token.
/// Returns None on missing token (macOS Keychain), 401, or network error — caller
/// falls back to the bundled catalog.
async fn fetch_model_catalog(access_token: &str) -> Option<Vec<CatalogModel>>;

struct CatalogModel { id: String, display_name: String, /* max_input_tokens, capabilities … */ }
```
- Clone the request shape from `key_validator.rs:141-163`; swap the header to `Authorization: Bearer`.
- Read the token via a shared helper factored out of `cli_handlers.rs:344-369` (move the
  `claudeAiOauth.accessToken` parse into `identity/resolver.rs` so both the auth check and this fetch use
  one code path).
- Map `data[]` → `CatalogModel`. **Family-alias mapping:** the CLI's `--model` still takes the family
  alias, so keep `value = "sonnet"` and set `label` from the API's `display_name` for the concrete model
  the alias currently resolves to. (Determining "which concrete model does `sonnet` resolve to on this CLI
  version" is the one thing the API doesn't tell us directly — see Open Questions Q1.)

### 4.2 Backend — cache + `ProviderConfig.models`
- Add a `models` field to Rust `ProviderConfig` (`providers.rs`; currently absent — this is the SPEC
  Open-Q1 from the dropdown spec, now resolved: yes, mirror into Rust so the catalog can be
  server-owned/refreshable rather than a compile-time TS constant).
- **Cache** the fetched catalog server-side keyed per `(provider, cli_version)` — a store row or a JSON
  file under the data dir (mirror how existing per-instance state persists). Ship the curated list as the
  **seed/fallback** value so a fresh install has a correct-enough catalog before the first fetch.
- Expose a `providers.models` RPC (or extend the existing provider-config RPC) that returns the cached
  catalog. **Never** fetch synchronously on this RPC — always serve cache; refresh is out-of-band (§4.3).

### 4.3 Backend — refresh task (reuse, don't invent)
- **Primary trigger:** on CLI **install/upgrade** — AgentMux already owns `npm install
  @anthropic-ai/claude-code@<pin>` and `claude update` (`install_handlers.rs` / `cli_handlers.rs`). After a
  successful install and once a valid token exists, kick a one-shot `fetch_model_catalog` and update the
  cache. This is the cadence `SPEC_CONTEXT_VISIBILITY_2026_06_17.md:142-147` chose.
- **Optional lazy refresh:** a "cache older than ~30 days AND token valid" check — model it on a plain
  `tokio::spawn` interval like `agentmux-cef/src/memory_heartbeat.rs`, or, if we want persistence +
  catch-up/misfire discipline, copy the pattern from `backend/cron/mod.rs` (`CronScheduler`). Do **not**
  build a general periodic-Rust-job framework for this one task.
- On 401/expiry, keep last-good cache and retry on next auth event.

### 4.4 Frontend — overlay + converge dropdowns
- `providers/index.ts`'s hardcoded `models` (`:196-200`) becomes the **fallback default**, overlaid at
  runtime by the cached catalog from `providers.models`.
- **Converge all three consumers to read the registry** (as `/model`'s `modelChoices` already does,
  `runtime.ts:64`): `AgentComposerStrip.tsx` and `AgentControlBar.tsx` must stop hardcoding their lists.
  (PR #1912 already routes the strip's Model select through `getProvider(providerId)?.models` and removes
  `AgentControlBar`'s duplicate — so post-#1912 there's effectively one consumer plus `/model`, and this
  step just points the registry read at the server-overlaid catalog.)
- Result: **"Sonnet 5" appears automatically** from `display_name`, with no hand-edited label to drift.

## 5. Optional — Haiku's legitimate role (validation, not enumeration)
Not the mechanism, but a nice-to-have hardening: a cheap best-effort probe that **validates** the catalog
against the installed CLI (e.g. a `--model <id> --print` dry-probe, or annotate against `claude --help`),
logging drift ("catalog lists X but the installed CLI rejects it"). This is the `B.3.2` validator from
`SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md:78` — reuse `invoke_cli_for_activity`
(`app_api/session.rs:198`) as the spawn primitive if built. Ship curated+API first; add the validator as a
follow-up.

## 6. Open questions
1. **Alias→concrete resolution.** The API lists all models but doesn't state which concrete model the
   `sonnet` *alias* resolves to on CLI version N. Options: (a) label the alias entry with the highest
   `display_name` in the family from `/v1/models` (assumes the alias tracks latest — usually true);
   (b) offer **concrete** IDs (`claude-sonnet-5`, `claude-sonnet-4-6`) as distinct picks so the user
   chooses the exact version (this is what lets a user pick "Sonnet 4.6" *vs* "Sonnet 5" —
   `SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG B.1`); or (c) both — a "Sonnet (latest)" alias entry plus concrete
   version entries. **Recommend (c).**
2. **macOS Keychain token.** No file token there → always fall back to bundled catalog on macOS, or add a
   `claude`-CLI-mediated fetch path. Accept the fallback for v1.
3. **Where to cache.** Store row vs JSON-under-data-dir; per-`(provider, cli_version)` key either way.
4. **Context windows too?** `/v1/models` also returns `max_input_tokens` — fold context-window refresh
   into the same task (aligns with `SPEC_CONTEXT_VISIBILITY_2026_06_17`), or keep this label-only for v1.
   Recommend label-only first, context-window as a fast follow that shares the fetch.

## 7. Test plan
- Unit: `fetch_model_catalog` maps a stubbed `/v1/models` `data[]` to `CatalogModel[]`; returns `None` on
  401 / missing token / network error.
- Integration: after a stubbed CLI install, the cache is populated; `providers.models` RPC returns it;
  frontend registry overlays it and the strip/`/model` dropdowns show the API labels.
- Fallback: with no token (simulated macOS Keychain), the dropdown shows the bundled curated list, no error.
- Live smoke (real OAuth): confirm `GET /v1/models` returns 200 with `Authorization: Bearer` + no beta
  header; confirm "Sonnet 5" surfaces.
- Convergence: assert `AgentComposerStrip`, `AgentControlBar` (post-#1912), and `/model` render the same
  list.

## 8. Sources
- Credential extraction: `agentmux-srv/src/server/cli_handlers.rs:344-369` (verified); macOS Keychain
  caveat `:371-378`; `identity/resolver.rs:131-143`.
- Outbound Models API HTTP: `agentmux-srv/src/identity/key_validator.rs:141-163` (verified).
- OAuth reachability + caching guidance: `docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md:134,136,138-147`.
- Curated catalog + registry: `frontend/app/view/agent/providers/index.ts:36-42,185-200`;
  `agentmux-srv/src/backend/providers.rs:33-83,147`.
- Dropdown consumers / drift: `commands/global/runtime.ts:58-72`;
  `docs/specs/SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md:39-42,46,71,73,78`.
- CLI version detection: `cli_handlers.rs:635-675,734-753`.
- Reuse patterns: `agentmux-srv/src/backend/cron/mod.rs` (scheduler);
  `agentmux-cef/src/memory_heartbeat.rs` (interval loop); `app_api/session.rs:198-267`
  (`invoke_cli_for_activity`, the Haiku spawn primitive — for §5 validation only).
