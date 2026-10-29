# Free and local options for the 19 catalog providers

**Status:** analysis — the research behind `docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md`; claims verified 2026-10-29 and going stale from then.

Task #10963 (research). Written 2026-10-29. It ships with the first PR of the build that cites it.

## How to read this

Every claim below was read on the web on **2026-10-29**. Six research agents fetched the pages. Their fetch tool returns a small model's summary of each page, so a quote is only as exact as that summary. I re-read these claims on the raw page myself:

- Gemini CLI's README (the personal free tier moved to Antigravity).
- Codex's pricing line.
- x.ai/build ("Available to try for Free").
- Antigravity's pricing page ("Basic weekly rate limits").
- Kilo's gateway doc ("200 requests per hour per IP").
- OpenRouter's limits page.

Anything I did not or could not confirm is marked **unverified** and says why. I tested nothing that costs money, and I used no paid or free account on any provider. Two things I did test myself, with no spend, are in the Codex section.

**What "Free" means here:** a beginner can use the provider with no subscription and no money, through the vendor's own sign-in or key, and no hardware beyond a normal laptop. "Free if your machine can run it" (local models) is listed separately, as you asked.

## Answers first

1. **Does Codex have a free option?** Maybe, and I can't confirm it. OpenAI's pricing page says "ChatGPT Work and Codex are included in your ChatGPT Free, Go, Plus, Pro, Business, Edu, or Enterprise plan". But that page lists "Codex on the web, in the CLI, in the IDE extension" only under Plus, and the Free and Go cards say "GPT-6 Luna at Standard speed in the desktop app, subject to rollout". Whether `codex` CLI sign-in works on a Free account needs a real Free account to settle. The repo README says "We recommend signing into your ChatGPT account to use Codex as part of your Plus, Pro, Business, Edu, or Enterprise plan", which points the other way. **No Free ribbon until someone tests it with a Free account.**
2. **Can Codex use other API keys?** Yes, but only for endpoints that speak the **Responses API**. Chat Completions support was removed (see below). In practice:
   - **Ollama and LM Studio, local:** verified. Both serve `/v1/responses`. Ollama's is stateless only; LM Studio's docs name Codex as supported. Codex has built-in `--oss` and `--local-provider ollama|lmstudio`.
   - **Groq:** documents a Responses API, but I did not see Codex run against it.
   - **OpenRouter:** has a beta, stateless Responses API. Whether it works with `:free` models is unverified.
   - **Gemini, Mistral, Cerebras, NVIDIA:** no Responses API documented, so no Codex.
   - So the only free route I can call verified for Codex is local.
3. **Local models.** Ollama can be serving with **no models pulled** (a fresh install), and LM Studio's `lms` CLI can be installed without a model. Only Mux Code lists `ollama` in `supportedVendors` today. 16 of the 19 providers can talk to a local model through some config; 3 cannot (Gemini CLI, Antigravity, Muse). See the table and the wiring list.

## What I tested myself (Codex 0.166.0, scratch install, no spend)

- `wire_api = "chat"` in `config.toml` is rejected: "`wire_api = "chat"` is no longer supported" (discussion https://github.com/openai/codex/discussions/7782). Only `"responses"` works.
- A custom provider in `[model_providers.mock]` with `name`, `base_url`, `env_key` and `wire_api = "responses"` worked against a local mock server: the request was `POST /v1/responses` with `Authorization: Bearer <the env_key's value>`. The mock server is stopped.
- Flags `--oss`, `--local-provider lmstudio|ollama` and `-c key=value` exist.
- An unknown model gives a "Model metadata ... not found" warning but still runs.

So wiring a key for Codex is real and small: AgentMux writes a `config.toml` into the agent's `CODEX_HOME` and passes the key through the variable named in `env_key`, from the Vault.

## Table: the 19 catalog providers

Verdict key: **FREE** = vendor offers a no-payment route inside the product (with catches); **KEY** = free software, no free model of its own, but it works with a free key from another provider; **LOCAL** = free if your machine runs the model; **NO** = paid only.

| Provider | Verdict | Free route today | Limits | Card / phone | Biggest catch |
|---|---|---|---|---|---|
| Claude Code | NO | None from Anthropic. The free claude.ai plan has no Claude Code ("The free claude.ai plan does not include Claude Code access.") | n/a | n/a | Pointing it at free non-Claude models works but is unsupported ("doesn't support routing Claude Code to non-Claude models through any gateway") |
| Codex CLI | UNCONFIRMED | ChatGPT Free "includes" Codex, CLI not confirmed. LOCAL via `--oss` is verified | Free quota unpublished | ChatGPT account; card and phone unverified | Pricing page and README disagree. Responses API only |
| Mux Code | LOCAL | Local GGUF via llama-server, or any OpenAI-compatible URL | Your hardware | None | Default local model `qwen2.5-coder:7b` is weak at tool use; needs Node 24.18.1+ |
| Gemini CLI | FREE (key only) | Free Gemini API key from AI Studio, Flash models only. The personal-account free tier **ended 2026-06-18** | 250 requests/day (official quota page) | Google account; card and phone unverified | Free-tier data is "used to improve our products". The CLI itself is "Not optimized for Gemini 4+" |
| Qwen Code | KEY / LOCAL | The Qwen OAuth free tier was discontinued 2026-04-15 | n/a | n/a | Needs OpenRouter `:free`, Ollama, or similar |
| OpenClaw | KEY / LOCAL | None of its own ("No subscription. No hosted tier. No token.") | Your provider's | Your provider's | A messaging-app assistant, not a coding CLI |
| Kimi Code | NO | None. Kimi Code is a membership benefit; the API needs a $1 minimum recharge | n/a | Card for any paid route | Don't confuse with the archived Python `kimi-cli` |
| GitHub Copilot CLI | FREE | Copilot Free plan: "All plans include Copilot CLI and the Copilot app." Auto model only | Small AI-credit allowance, amount unpublished; 2000 completions/month | GitHub account; card and phone unverified | Individuals only; cloud agent not on Free |
| Pi | KEY / LOCAL | None of its own; many built-in providers | Your provider's | Your provider's | Repo and npm names moved (`earendil-works`) |
| Antigravity (agy) | FREE | $0 Individual plan, personal Google account: "Basic weekly rate limits" | Weekly quota, number unpublished; Gemini Flash-class models | Google account; card and phone unverified | No BYOK. Free-tier data terms unverified |
| Kilo Code CLI | FREE | Hosted `:free` models and Auto Free through the Kilo gateway, anonymous or signed in | 200 free-model requests/hour/IP | "You do not need an API key or credit card for these free models." Phone unverified | Lineup changes ("hosted promotions can end"); some free models require data collection |
| Cline CLI | FREE (rotating) | Models tagged FREE, with a Cline account | Quota unpublished | Account via Google, GitHub or email; card unverified | "Rotating, limited-time basis"; usage "may be used to help improve model performance" |
| OpenCode | FREE (rotating) | Free models through OpenCode's own gateway (`opencode/<model>`) | No number published. Users report hitting a wall | Free-only sign-in rules unverified | Free models only work in the OpenCode client; several may train on data; lineup rotates |
| goose | KEY / LOCAL | None of its own. Docs cover Groq, Gemini free tiers and local models; Tetrate gives $10 credit once | Provider's | Provider's | Models without tool calling can only chat |
| Grok Build | FREE | x.ai/build says "Available to try for Free" with a grok.com or X account | Unpublished. The Grok 4.5 free promo is "for a limited time" and may not apply to 4.7 | Unverified | The launch post said SuperGrok / X Premium Plus only; the product page now says Free. Treat limits as unknown |
| DeepSeek Harness (dsh) | KEY / LOCAL | The harness is free (MIT). DeepSeek's API is pay-as-you-go | n/a | n/a | A signup grant exists ("granted balance") but its size, expiry and card rule are unverified; developer preview |
| Aider | KEY / LOCAL | Free tool; works with a free Gemini key, OpenRouter `:free`, or Ollama | Provider's | Provider's | **Stale:** last release v0.86.0 on 2025-08-09; its doc still advertises the old `gemini-exp` route |
| Crush | KEY / LOCAL | None. The official provider (Charm Hyper) is "subscription-based" | n/a | n/a | FSL-1.1-MIT licence (source-available); can't use OpenCode's free models |
| Muse Code | NO | None. Subscription tiers or pay-per-token API | Lowest tier "10-50 prompts every 5 hours" | Meta account, 18+; card unverified | "Contributor mode" is cheaper only in exchange for training on your data. No local-model support |

### Per-provider details I have: signup URLs and first steps

These are the first steps AgentMux's guide would show.

- **Antigravity:** install (`irm https://antigravity.google/cli/install.ps1 | iex` on Windows, or `curl -fsSL https://antigravity.google/cli/install.sh | bash`), run `agy`, sign in with a Google account in the browser, `/usage` shows the quota. https://antigravity.google/docs/getting-started?tab=cli
- **Copilot CLI:** pick the Free plan at https://github.com/features/copilot, install per https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli, run `copilot` then `/login` (browser or device code). I did not verify the install command.
- **Grok Build:** `irm https://x.ai/cli/install.ps1 | iex` (or `curl -fsSL https://x.ai/cli/install.sh | bash`), run `grok`, the browser opens to sign in; headless: `grok login --device-auth`. https://x.ai/build
- **Gemini CLI (key route):** create a key at https://aistudio.google.com/apikey, set `GEMINI_API_KEY`, run `gemini`, choose "Gemini API Key". I believe the variable name is `GEMINI_API_KEY`; the README text I fetched did not state it.
- **OpenCode:** `curl -fsSL https://opencode.ai/v2/install | bash` or `npm i -g @opencode/cli`, run `opencode`, `/models`, pick one tagged free (e.g. `opencode/big-pickle`). https://opencode.ai/auth . The v1 and v2 CLIs differ in config syntax; the sign-in-free claim is unverified for v2.
- **Kilo:** `npm install -g @kilocode/cli`, run `kilo`, `/connect`, pick Auto Free. https://kilo.ai (sign-in at https://app.kilo.ai). Whether the CLI works fully signed out is unverified.
- **Cline:** `npm i -g cline`, `cline auth`, run `cline`, set the provider to Cline, choose a model tagged FREE. https://app.cline.bot

## Free keys that work with the "KEY" tools (Aider, goose, Pi, Qwen Code, Crush, OpenClaw, Kimi, dsh, Kilo, Cline, OpenCode)

| Route | Signup | Limits | Card | Catches |
|---|---|---|---|---|
| Google AI Studio (Gemini API free tier) | https://aistudio.google.com/apikey, Google account | Not published in the docs ("Specified rate limits are not guaranteed"; see AI Studio). The gemini-cli quota page says 250/day for a key | "New accounts begin on the Free Tier"; phone unverified | Free-tier content is used to improve Google products outside the EEA, Switzerland and the UK. OpenAI-compatible endpoint: `https://generativelanguage.googleapis.com/v1beta/openai/` |
| OpenRouter `:free` models | https://openrouter.ai | 20 requests/min; 50/day if you have bought under 10 credits ever, 1000/day at 10 or more (https://openrouter.ai/docs/api-reference/limits) | Not stated for the 50/day tier; unverified | A negative balance breaks even free models; a `:free` model inside a fallback list can fall back to a paid one; 16 free ids today and the list churns; 50 requests is minutes of agent work |
| Groq free plan | https://console.groq.com/keys | gpt-oss-120b/20b and qwen3.8-27b: 30 req/min, 1000/day, 8K tokens/min, 200K tokens/day (column order read from a data table, likely) | Upgrading to the Developer tier needs a payment method, so the free tier doesn't | 8K tokens per minute is tight for an agent that re-sends a big prompt each turn |
| NVIDIA build.nvidia.com | https://build.nvidia.com | Not published; a forum post shows 1000 credits and ~40 req/min | No; phone unverified | "for prototyping, research, development and testing purposes only"; logged for product improvement |
| Mistral free mode | https://console.mistral.ai | "Usage and rate limits apply", no numbers | "API access is enabled by default with no credit card required" | Current training policy on free usage unverified |
| Cerebras | https://cloud.cerebras.ai | $5 one-time credit, expires in 30 days | **Card required** | "doesn't currently offer a no-cost tier that renews automatically" |

**Not free, despite what blog posts say:** GitHub Models ("fully retired" as of 2026-07-30), Hugging Face Inference Providers (free users have no monthly credits), Ollama cloud models ("Pay per token for every cloud model"). **Not checked:** SambaNova, Together AI.

## Local models: "free if your machine can run it"

A fresh Ollama install serves on 127.0.0.1:11434 with **no models** (`/api/tags` returns `{"models":[]}`). I pulled nothing and did not test any model. Sources for the model facts are the Ollama library pages, fetched 2026-10-29.

### Models to pull, and sizes

| Pull | Download | Context | Tools | Licence | Why |
|---|---|---|---|---|---|
| `qwen3-coder:30b` | 19 GB | 256K | yes | Apache 2.0 | A mixture-of-experts model with ~3B active parameters, so it is usable on CPU. First pick. Open tool-call parser bugs exist on Ollama (#16686, #18530): expect an occasional dropped call |
| `gpt-oss:20b` | 14 GB | 128K | yes | Apache 2.0 | Lightest and the one most agent CLIs are tested against. Needs about 16 GB of memory |
| `devstral-small-2:24b` | 15 GB | 384K | yes | Apache 2.0 | Best published agentic-coding score of the three (68.0% on SWE-bench Verified per its card), but dense, so slow on CPU. Worth it only with a GPU |

I did not evaluate newer entries on the library (`qwen3.8:27b`, `north-mini-code-1.0`, `laguna-xs-2.1`, `ornith:9b`, `muse-glimmer:30b`). Don't recommend `qwen2.5-coder` or the deepseek-coder variants for agent work; I didn't open their pages, so "older" is unverified. Mux Code's default `qwen2.5-coder:7b` falls in that group.

**The setting that bites agents:** Ollama's default context is 4K under 24 GB of VRAM (32K at 24-48 GB). Its docs say coding tools "should be set to at least 64000 tokens". `OLLAMA_CONTEXT_LENGTH=64000 ollama serve`, and check with `ollama ps`. Codex and Claude Code pages on Ollama both say 64K+. I did not check whether the 4K default also applies on a CPU-only machine; assume yes and set it explicitly.

**Speed on a machine with 64 GB of RAM: unverified.** My guess is the MoE models are usable on CPU and the dense 24B model is single-digit tokens per second, but I haven't measured either. Prompt processing of large agent prompts will probably be the pain on CPU. Whether the test machine has a GPU, I did not check.

### Per provider: (a) what the CLI supports, (b) how well it works, (c) what AgentMux writes

Practice (b) is from the docs and the providers' own issue trackers. **I ran none of these against a local model**, so treat (b) as reported, not measured.

| Provider | (a) CLI support | (b) In practice | (c) What AgentMux would write |
|---|---|---|---|
| Claude Code | Yes via `ANTHROPIC_BASE_URL`. Ollama and LM Studio both serve an Anthropic Messages endpoint (`/v1/messages`) | Works with rough edges (open issues on tool scaffolding being misread, an empty-block replay). Anthropic does not support it | Env: `ANTHROPIC_BASE_URL=http://localhost:11434` (LM Studio: `:1234`), `ANTHROPIC_AUTH_TOKEN=ollama`, `ANTHROPIC_API_KEY=""`, `--model <tag>`. `baseUrlEnvVar` already exists for claude; add the auth-token and empty-key vars |
| Codex | `--oss` / `--local-provider ollama\|lmstudio`, `oss_provider` in config, or a `[model_providers.x]` table | The roughest fit: maintainers say frontier models get "almost all of the team's attention". Open issues: MCP tools wrapped in a `namespace` type Ollama and LM Studio don't handle (#26234), `--oss` with LM Studio never reaching `/v1/responses` (#35758), `spawn_agent` failing on custom providers (#42488) | `config.toml` in `CODEX_HOME`: `oss_provider = "ollama"` or a `[model_providers.x]` with `wire_api = "responses"`, `base_url`, `env_key`; context 64K+ |
| Mux Code | Local GGUF through llama-server, or `-b openai-compat --base-url <url>` (point at Ollama or LM Studio `/v1`) | No quality data. Default model is weak | Already has the `ollama` vendor. Add LM Studio as a second vendor; pass `--base-url` |
| Gemini CLI | **No.** Only Google login, a Gemini key, or Vertex. An OpenAI-compatible request was closed ("we don't have immediate plans"). `GOOGLE_GEMINI_BASE_URL` exists but only redirects the Gemini API client | Not a fit | Nothing. Not wirable without a translating proxy |
| Qwen Code | Yes: `OPENAI_BASE_URL`, `OPENAI_API_KEY`, `OPENAI_MODEL`, or `modelProviders.openai[]` in `settings.json`. The README names Ollama and vLLM | Open Ollama-specific bugs (zero-argument tools rejected, #12878; full re-processing each step, #14203) | Env vars (or a `settings.json` with `security.auth.selectedType: "openai"`) |
| OpenClaw | Yes, through the native Ollama API (`baseUrl: "http://host:11434"`) | Its own docs: don't use the `/v1` URL, it "breaks tool calling" | `models.providers` entry with the native URL, not `/v1` |
| Kimi Code | Probably: `config.toml` `[providers.x]` with `type = "openai"` and `base_url`. The Ollama case is from a fetch summary | No data | A `config.toml` provider block; keys only through `api_key_env` (shell env is not imported) |
| Copilot CLI | Yes (BYOK). Auto-discovers Ollama at 127.0.0.1:11434; or `~/.copilot/providers.json` | No data. The model must support tools and streaming | Nothing for Ollama on the default port (just set `OLLAMA_HOST` if not); otherwise `providers.json`; `COPILOT_OFFLINE=true` to stop contact with GitHub |
| Pi | Yes: `models.json` with `api: "openai-completions"` and `baseUrl http://localhost:11434/v1` | No data | A `models.json` provider entry |
| Antigravity | **No** for the CLI ("no support for ... bring-your-own-key or bring-your-own-endpoint"). The Python SDK can, the CLI cannot | n/a | Nothing |
| Kilo Code | Yes (an OpenCode fork, same config): `provider.<id>` in `kilo.jsonc`. Ollama doc says `num_ctx` at least 32K | No Kilo-specific local issues found; expect OpenCode's behaviour | A `kilo.jsonc` provider entry |
| Cline | Yes (Ollama, LM Studio, OpenAI-compatible). CLI flags `cline auth -p <provider> -k <key> -b <baseurl> -m <model>` | Unverified for the CLI; the Ollama guide is written for the extension and says enable "Use Compact Prompt" | `cline auth` flags or `providers.json`; exact local provider id unverified |
| OpenCode | Yes: `@ai-sdk/openai-compatible` with `baseURL http://localhost:11434/v1`; v2 auto-discovers Ollama at 127.0.0.1:11434 | The docs warn "there are only a few of them that are good at both generating code and tool calling". Open issues on subagents receiving no tool definitions (#51268) and malformed repeated tool calls (#59032, 2026-10-28) | `opencode.json` / v2 `opencode.jsonc` provider block (v1 and v2 syntax differ) |
| goose | Yes: Ollama (`OLLAMA_HOST`), LM Studio (`http://localhost:1234`), OpenAI-compatible (`OPENAI_HOST`, `OPENAI_BASE_PATH`) | "goose extensively uses tool calling, so models without it can only do chat completion". Open Ollama issues: stall timeout aborts long tool calls (#13090) | Env vars, or a JSON in `custom_providers/` |
| Grok Build | Yes: `[model.<name>]` in `~/.grok/config.toml` with `base_url`, `api_backend`, `context_window` | No independent data. `stream_tool_calls` may need opting out per model | `config.toml` model block |
| DeepSeek Harness | Yes: `ollama launch dsh`, or a provider in `cordis.patch.yml` (second source only) | Unverified. Developer preview | Via `ollama launch dsh --model <tag>` or a profile file; official dsh docs would not load |
| Aider | Yes: `OLLAMA_API_BASE`, `--model ollama_chat/<tag>`; LM Studio: `LM_STUDIO_API_BASE`, `LM_STUDIO_API_KEY=dummy-api-key`, `lm_studio/<tag>` | The most robust of the list for small models: it edits with text formats, not tool calls. It auto-sets `num_ctx`. But it is not an agent loop and the project looks stale (last release 2025-08-09) | Env vars and `--model` |
| Crush | Yes: `provider add ollama --type ollama --base-url http://localhost:11434/v1`, `--type completions` for others | No data | A `crush.json` providers block; exact field names not confirmed |
| Muse Code | **No** in anything I opened | n/a | Nothing |

**Summary:** 16 can be pointed at a local model, 3 cannot (Gemini CLI, Antigravity, Muse). The ones I would call best supported by the CLI's own docs: OpenCode, Kilo, Qwen Code, Pi, Grok Build, Copilot CLI, goose, Aider. Claude Code and Codex work only through unsupported or fragile paths.

### What AgentMux would have to build for Local

Today only `claude` has `baseUrlEnvVar`, and only Mux Code has `supportedVendors` with `ollama`. A generic design:

1. A detector: is Ollama on 127.0.0.1:11434 (`/api/tags`), is LM Studio on `:1234` (`/v1/models`); which models are present; is `lms` installed. Shown in the new-agent form as "Local models available: qwen3-coder:30b".
2. A "Model source" choice in the create form for providers whose `localSupport` is set: Cloud (default), Local (Ollama / LM Studio), Custom endpoint.
3. Per provider, a small writer in `buildConfigFiles` or the env builder: the right-hand column above. The files are `config.toml` (Codex, Kimi, Grok), JSON (OpenCode, Kilo, Pi, Cline, goose, Crush, Copilot, Qwen), and env vars (Claude, Aider, Qwen, goose).
4. The context length: AgentMux cannot set it for an Ollama it doesn't own, so the form says "set `OLLAMA_CONTEXT_LENGTH=64000`" and `ollama ps` shows what is loaded; checking it is a cheap probe worth building.

## Recommendation: which providers get the blue "Free" ribbon

**Principle:** Free appears only if a person can start with the vendor's own sign-in or key, pay nothing, and use the product for real. A tool that is merely free software with no free model gets no Free tag.

**Give Free (7):**

| Provider | Why | What the tooltip must say |
|---|---|---|
| Antigravity | Official $0 plan, Google account only | Weekly quota; number not published |
| GitHub Copilot CLI | Official Free plan includes the CLI | A small credit allowance; auto model only |
| Grok Build | Official "Available to try for Free" | Limits unpublished; a free promo may end |
| OpenCode | Official free models in the client | Rotating; some free models may train on your data |
| Kilo Code | Official free models, no card, documented limit | 200 requests/hour; lineup changes |
| Cline | Official free models with an account | Rotating promotion; quota unpublished |
| Gemini CLI | Free Gemini API key works natively | 250 requests/day, Flash only, trains on your data outside the EEA |

**Do not give Free:** Claude Code (no), Codex (unconfirmed; revisit after a test with a real Free account), Kimi Code, Muse Code, Crush (all paid), and Qwen Code (its free tier is gone).

**Give Local (see the next spec), not Free:** Mux Code, plus the 15 CLIs above that can reach a local model. Free software with a free key (Aider, goose, Pi, OpenClaw, dsh, Qwen Code, Crush) is a third idea, "works with a free key". I'd show it only in the tooltip and in the signup guide, not as a tag, so the blue Free ribbon stays honest.

## Best 3 to 5 for a beginner

1. **Antigravity.** One Google sign-in, no key, no card as far as the pages say. Catch: quota unpublished, and a blog claim that it lasts 20-30 minutes of real work is unverified.
2. **GitHub Copilot CLI (Free plan).** A GitHub account most developers already have; real but small allowance, auto model only.
3. **Grok Build.** Official "try for free", a normal account sign-in. The weakest on facts: limits and promo end are unknown.
4. **OpenCode or Kilo Code.** No card, many models. Kilo has the only documented number (200 requests/hour/IP). OpenCode's free-only sign-in rules are unverified.
5. **Local: Ollama with `qwen3-coder:30b`**, if the machine has about 16+ GB of RAM and the person is comfortable with a 19 GB download. Private and unlimited, but the quality is well below a cloud model.

## What I could not verify (so nothing here should be quoted as fact)

- Whether Codex CLI works on a ChatGPT Free account.
- Card and phone requirements: Gemini API key, OpenRouter free, Groq, Mistral, NVIDIA, Antigravity, Copilot Free, Grok, Cline.
- The size of any free quota for Antigravity, Copilot Free, Grok, Cline, OpenCode, and per-model Gemini limits.
- DeepSeek's signup grant (amount, expiry, card rule).
- Free-tier training terms for Antigravity, Copilot Free, Grok and OpenRouter free models.
- Muse Code's plan prices (from Meta's own page; I only have a third-party figure).
- Whether Codex runs well against OpenRouter `:free` or Groq.
- Local model speed on a given machine, and whether it has a GPU.
- Whether OpenCode free models work signed out on v2 (docs conflict).

## Things that will go stale

Anything marked "rotating", "limited time" or "promotion" above. The Gemini story already flipped on 2026-06-18 and many blogs still show the old one (including the quota table on geminicli.com, which shows "1,000 requests" next to its own banner saying the unpaid tier was replaced). This is why #10964 needs a data file with a "verified on" date per claim, not text baked into the UI.

## Sources (all fetched 2026-10-29)

- Gemini CLI README and quota page: https://github.com/google-gemini/gemini-cli , https://geminicli.com/docs/resources/quota-and-pricing/ , announcement https://github.com/google-gemini/gemini-cli/discussions/28017
- Gemini API pricing: https://ai.google.dev/gemini-api/docs/pricing
- Antigravity: https://antigravity.google/pricing , https://antigravity.google/docs/plans , https://antigravity.google/docs/getting-started?tab=cli
- Grok Build: https://x.ai/build , https://x.ai/news/grok-4-5 , https://github.com/xai-org/grok-build
- Codex: https://learn.chatgpt.com/docs/pricing , https://learn.chatgpt.com/docs/config-file/config-reference , https://github.com/openai/codex , https://docs.ollama.com/integrations/codex
- Claude Code: https://code.claude.com/docs/en/setup , https://code.claude.com/docs/en/llm-gateway , https://docs.ollama.com/integrations/claude-code
- Copilot: https://docs.github.com/en/copilot/get-started/plans , https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models
- OpenCode: https://opencode.ai/v2/docs/console/models/ , https://opencode.ai/v2/docs/models/
- Kilo: https://kilo.ai/docs/gateway/models-and-providers , https://kilo.ai/landing/free-models
- Cline: https://docs.cline.bot/getting-started/free-models
- goose: https://goose-docs.ai/docs/getting-started/providers/
- Qwen: https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/
- Kimi: https://www.kimi.com/code/docs/en/ , https://platform.kimi.ai/docs/pricing/limits
- Muse: https://dev.meta.ai/docs/muse-code/subscriptions , https://dev.meta.ai/docs/pricing-rate-limits
- OpenRouter: https://openrouter.ai/docs/api-reference/limits
- Groq, Cerebras, Mistral, NVIDIA, Cloudflare, GitHub Models, Hugging Face: https://console.groq.com/docs/rate-limits , https://www.cerebras.ai/pricing , https://docs.mistral.ai/admin/billing-usage/usage-limits.md , https://docs.api.nvidia.com/nim/docs/product , https://developers.cloudflare.com/workers-ai/platform/pricing/ , https://docs.github.com/en/github-models/prototyping-with-ai-models , https://huggingface.co/docs/inference-providers/pricing
- Ollama and LM Studio: https://ollama.com/library/qwen3-coder , https://ollama.com/library/gpt-oss , https://ollama.com/library/devstral-small-2 , https://docs.ollama.com/context-length , https://docs.ollama.com/api/openai-compatibility , https://docs.ollama.com/api/anthropic-compatibility , https://lmstudio.ai/docs/developer/openai-compat , https://lmstudio.ai/docs/developer/anthropic-compat
- Mux Code, Aider, OpenClaw, Pi, dsh, Crush: https://github.com/agentmuxai/muxcode , https://aider.chat/docs/llms/ollama.html , https://docs.openclaw.ai/providers/ollama , https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/models.md , https://github.com/deepseek-ai/deepseek-harness , https://github.com/charmbracelet/crush
