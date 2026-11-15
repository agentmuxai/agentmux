# Spec: Nine new harnesses — OpenCode, Kilo Code, Cline, goose, Grok Build, DeepSeek Harness, Muse Code, Crush, Aider

**Status:** active. Shipped: Phase 0 (#4889, #4890, #4893, #4895), the six ACP harnesses (#4899, #4908), the terminal tier with Aider and Crush (#4926), Muse Code over MSP (#4932, §7.7), and OpenCode's token usage (#4930) and context meter (§5 item 4). Remaining: token usage and the context meter for the other ACP agents, once each is measured, and a check of Muse Code's tool calls and approvals signed in.
**Date:** 2026-10-14
**Author:** Lazo

## 1. Ask

The owner asked to add every major coding-agent harness AgentMux doesn't have yet, with a complete analysis of everywhere a harness is bound: this repo, the docs site, the Toolchain pane, the version drift report and every other place.

AgentMux has ten today: Claude Code, Codex, Gemini CLI, Antigravity, Qwen Code, Kimi Code, Pi, OpenClaw, GitHub Copilot CLI and muxcode. The nine below are the widely used ones still missing, as of 2026-10-14.

## 2. The nine

Facts are from each project's repo, docs, release page and package registry, checked on 2026-10-14. Stars are GitHub's count that day.

| Harness | Upstream | Stars | License | Install | Pin candidate | How AgentMux would drive it |
|---|---|---|---|---|---|---|
| OpenCode | anomalyco/opencode | 213k | MIT | npm `opencode-ai` (V1) or `@opencode/cli` (V2), native binaries per platform | `opencode-ai@1.19.0` (§7.1) | ACP: `opencode acp` |
| Kilo Code CLI | Kilo-Org/kilocode (an OpenCode fork) | 28k | MIT | npm `@kilocode/cli`, native binaries per platform | `7.8.8` | ACP: `kilo acp` |
| Cline CLI | cline/cline | 70k | Apache-2.0 | npm `cline`, native binaries per platform (Bun build) | `3.0.71` | ACP: `cline --acp` |
| goose | aaif-goose/goose (Linux Foundation; was block/goose) | 55k | Apache-2.0 | GitHub release binary (`goose-x86_64-pc-windows-msvc.zip`, …), curl/PowerShell script, Homebrew | `v1.54.0` | ACP: `goose acp` |
| Grok Build | xai-org/grok-build | 27k | Apache-2.0 | xAI installer (`x.ai/cli/install.sh`, `install.ps1`) or winget `xAI.GrokBuild`; no npm | stable `1.0.52` | ACP: `grok agent stdio` |
| DeepSeek Harness | deepseek-ai/deepseek-harness | 249k | MIT | npm `@deepseek-ai/dsh` (Node 22.19+) | `0.2.0-rc.2` (prerelease only) | ACP: `dsh --profile acp` |
| Muse Code | Meta, closed source (only the SDK is open: meta-models/muse-code-sdk) | — | proprietary | Meta installer (`dev.meta.ai/install.sh`, `install.ps1`); a launcher that downloads and auto-updates a ~450 MB native binary | stable `1.4.7-R6101.1` | No ACP. Meta's own protocol, MSP (`muse serve`, JSON-RPC over stdio), or `muse exec --json` (§7.7) |
| Crush | charmbracelet/crush | 29k | FSL-1.1-MIT (§7.8) | npm `@charmland/crush` (postinstall downloads the binary), winget, GitHub release | `v0.98.1` | No ACP yet (upstream PR open). Terminal tier (§4) |
| Aider | Aider-AI/aider | 50k | Apache-2.0 | Python: `uv tool install aider-chat`, pipx, or its uv-based install script | `0.86.2` (last release 2026-02-12) | No ACP, MCP or JSON output. Terminal tier (§4) |

Every one of them runs natively on Windows.

**Instructions files.**
- `AGENTS.md`: OpenCode, Kilo, Cline, goose, Grok Build, DeepSeek Harness, Muse Code and Crush.
  - Grok Build and Muse Code read project rules only in a trusted folder; see §7.5 and §7.7.
- Aider reads conventions only when they are passed with `--read <file>`.

**MCP.**
- Supported by all except Aider, each in its own config format:
  - `opencode.json` / `kilo.jsonc` `"mcp"`
  - Cline `cline_mcp_settings.json`
  - goose `config.yaml` `extensions`
  - Grok `config.toml` `[mcp_servers]`
  - DeepSeek profile YAML
  - Muse `settings.json` `mcp_servers`
  - Crush `crushrc`
- Over ACP, OpenCode and DeepSeek Harness also accept MCP servers in `session/new`.

## 3. How a harness is bound today

AgentMux has no generic "unknown CLI" path. A harness is bound in about 40 places, and about 15 of them must be filled in for it to launch at all.

**Required.**
- **Two registries that must match:**
  - `frontend/app/view/agent/providers/catalog.ts` `PROVIDERS`
  - `crates/srv/src/backend/providers.rs` `REGISTRY`
  - `pin-consistency.test.ts` enforces that the ids and launch args are equal.
- **Three alias tables:**
  - `providers.rs` `ALIASES`
  - `catalog.ts` `PROVIDER_ALIASES`
  - `providers/provider-id-aliases.ts`, test-enforced both ways.
- **The permission branch in `buildRuntimeArgs.ts`.** An unknown id falls into the Claude branch and gets `--dangerously-skip-permissions` appended. `pane-args-parity.test.ts` pins the set of providers that do.
- **The card**, an entry in `defaults/cli-catalog.ts` `CLI_CATALOG`. A missing entry defaults to offering Container mode.
- **The seeded definition.**
  - Add a row in `scripts/gen-seed.js` and regenerate `crates/srv/agent-seed.json`.
  - Raise its `version`, or the card never appears in the picker.
  - The generator has drifted: it lacks Antigravity, which the JSON has.
- **Auth:**
  - `identity/resolver/provider.rs` `provider_class`. The default `None` silently turns off account binding and env injection.
  - `identity/auth_patterns.rs`: an API-key provider must be listed in `is_api_key_provider`, or a login URL is misread as OAuth.
  - The catalog's `authType` / `authCheckCommand` / `authLoginCommand`. srv refuses an empty check command.
- **Startup instructions:**
  - `startup_instructions_filename` / `native_instruction_sources` in `providers.rs`
  - `startupInstructionsFilename` in the catalog.
  - The file must be one the CLI really reads.
- **Install:**
  - **npm harnesses** set `npmPackage` + `pinnedVersion` and get the managed, pinned install under `shared/cli/<provider>/<pin>`.
  - **Others** follow the Antigravity and Kimi precedent: `npmPackage: ""`, an install command shown as a hint, and srv finds the binary on PATH or in `known_install_paths`.
  - srv never runs a curl or pip installer, and `install.start` refuses an empty `npm_package`.

**The integration tiers that exist.**
- **ACP (`controllerType: "acp"`)**
  - Copilot, OpenClaw and Pi use it.
  - `blockcontroller/acp.rs` and `acp-translator.ts` work for any agent, so it needs no protocol code.
  - What's missing: token usage (`usage_update` is ignored), a model picker, AgentMux's MCP server (it sends `mcpServers: []`), and container mode.
- **Subprocess turns, reusing a translator.** muxcode emits `claude-stream-json`; Qwen has its own format name.
- **Subprocess with its own translator.** Codex, Gemini, Kimi and Antigravity.
- **Persistent with a control protocol.** Claude only.

The agent pane drops any output line that isn't JSON, so **a CLI with plain-text output has no tier.**

**CI gates a new provider trips.**
- `providers.rs` is at its 1890-line ceiling in `scripts/check-file-sizes.baseline`, and cannot grow.
- `all_providers_covers_every_registered_id_exactly_once` hard-codes 10 ids.
- Test-enforced lists:
  - `pin-consistency.test.ts`: pin equality for npm providers
  - `pane-args-parity.test.ts`: `KNOWN_STRAY_BYPASS_FLAG`, `KNOWN_MODELS_NOT_APPLIED`
  - `toolchain-catalog.test.ts`: local-tool ids must not collide with provider ids, and OpenCode, goose, Crush and Aider are already local-tool rows
  - `global-bundle-model.test.ts`: per-filename display-name lists
- `check-comment-hygiene.mjs`: foreign config paths such as `.goosehints` or `.opencode/` fail until allow-listed.

## 4. Approach

**ACP for the six that have it:** OpenCode, Kilo, Cline, goose, Grok Build and DeepSeek Harness.
- They need no protocol code.
- What each one needs is configuration and isolation: install source, sign-in, config-dir variable, startup file, permission mode, telemetry defaults.
- That is the per-harness work, and it is mostly data.

**A terminal tier for the two with no structured interface:** Aider, and Crush until its ACP lands.
- **What it is:** a harness whose card opens a terminal pane (`view: "term"`, `controller: "cmd"`) running the CLI's own TUI in the agent's folder.
- **What AgentMux still does:** writes the startup instructions there and, where the CLI supports it, passes them on the command line (`aider --read CONVENTIONS.md`).
- **What it doesn't get:** an AgentMux transcript, token count or resume. The CLI's own UI does those.
- **Why:** it is honest about what these CLIs offer, and it is far cheaper than a parser for unstructured text. The login flow already opens a terminal this way (`flows/login-backend.ts`).

**Muse Code needs a decision of its own** (§7.7).
- It has no ACP.
- Its documented, typed interface is MSP. MSP is ACP-like JSON-RPC over stdio, with item and delta notifications and token usage.
- `muse exec --json` streams JSONL, but its event schema is undocumented.
- The spike (2026-10-14) chose MSP: an `msp` controller, like the ACP one, drives `muse serve` in an agent pane. See §7.7.

**Isolation is verified against the real binary, not assumed.**
- Pi's `PI_HOME` and Antigravity's `ANTIGRAVITY_CONFIG_DIR` turned out to be variables the CLIs never read.
- So each harness PR checks, against the installed binary:
  - that the config-dir variable relocates its credentials and sessions;
  - that the startup file is read;
  - that ACP `initialize` and `session/new` succeed, signed out and signed in.

## 5. Phase 0: shared work, before any harness

One PR each.

1. **Split `providers.rs`.**
   - Move its tests (about 700 lines from line 1180) into `providers/tests.rs`, and the per-provider `ProviderConfig` statics into one file per provider or a `providers/registry.rs`.
   - Lower the size baseline with `check-file-sizes.mjs --update`.
   - Without this, no provider can be added.
2. **No Claude fallbacks for a non-Claude provider.**
   - `buildRuntimeArgs` adds no permission or model flags to an ACP provider. ACP permission is handled in the protocol: AgentMux auto-allows `session/request_permission`.
   - This closes the "stray bypass flag" gap that `pane-args-parity.test.ts` currently whitelists.
   - Generalise `prompt_arg_flag()`, today hard-coded to `antigravity`, into the registry field it mirrors.
3. **AgentMux's MCP server over ACP.**
   - Send `agentmux-mcp` in `session/new` / `session/load` `mcpServers` when the agent's `initialize` response advertises MCP capabilities. Fall back to `[]` otherwise.
   - This gives every ACP harness, including today's Copilot, OpenClaw and Pi, the MuxAPI tools (WhoAmI, messaging, memory). It is the follow-up `SPEC_ACP_CLIENT_CONFORMANCE_2026_10_07.md` left out of scope.
   - Without it, none of the nine has AgentMux's tools.
4. **Token usage over ACP.** Read the `session/prompt` response's `usage` into the turn's stats.
   - ACP leaves open whether it is per-turn or cumulative, and whether `inputTokens` includes the cache (agent-client-protocol#1860). So it is read only for an agent whose semantics were measured, through its own output format (`opencode-acp`, the way `qwen-stream-json` reuses the Claude translator).
   - OpenCode 1.19.0, measured: the turn's last model call; `inputTokens` is the uncached share, beside `cachedReadTokens`. Kilo Code, a fork, couldn't be measured signed out.
   - `usage_update` (`used` of `size`, and cost) is the context window's fill. OpenCode's feeds the context meter, on the model its `configOptions` name, with `size` as that model's reported window (`acp-context.ts`).
5. **A version source for non-npm harnesses.**
   - Add `versionSource?: { kind: "npm" | "pypi" | "github-release" | "vendor"; package?: string; repo?: string }` to `ProviderDefinition`. Its pin goes in `pinnedVersion` as for npm.
   - The version drift report reads `catalog.ts` and today monitors only `npmPackage`. A provider without one is reported as "unmonitored" and never flagged.
   - The report's side of this is planned in the private infrastructure repo.
6. **The terminal tier** (§4).
   - A `controllerType: "terminal"` launch path that opens a term block with the CLI and its args in the agent's folder, after writing the startup file.
   - The card's launch modal says the transcript lives in the CLI's own UI.
7. **Seed generator.**
   - Put Antigravity back into `scripts/gen-seed.js` `CLI_DEFS`, so regenerating doesn't drop it.
   - Add a check that `agent-seed.json` matches the generator, so they can't drift again.

## 6. Per-harness checklist

Every harness PR does all of these, and the PR body ticks them.

- [ ] `catalog.ts` `PROVIDERS` entry and `providers.rs` registry entry, in the same PR (ids, launch args, pin, `controllerType`, `outputFormat: "acp"` or the terminal tier, `authType`, check and login commands, `authConfigDirEnvVar`, `authDirName`, `startupInstructionsFilename`, `windowsInstallCommand` / `unixInstallCommand`, `versionSource`).
- [ ] Aliases in all three tables (e.g. `kilocode`, `opencode-ai`, `grok-build`, `dsh`).
- [ ] `buildRuntimeArgs` branch (or the Phase 0 ACP rule) and the matching `pane-args-parity` lists.
- [ ] `cli-catalog.ts` card: `displayName`, `blurb`, `primaryContextFile`, `mcpSupport`, `popoverMarkdown`, `hostSupported: true`, `containerSupported: false` (ACP and non-npm harnesses can't run in a container today).
- [ ] `gen-seed.js` row, regenerated `agent-seed.json`, `version` raised.
- [ ] `provider_class` arm (OAuth with its config-dir env var, or ApiKey with its variable), plus `auth_patterns.rs` (`is_api_key_provider` for key-only harnesses).
- [ ] `known_install_paths` for non-npm binaries (where each installer puts the binary on Windows, macOS and Linux), because a running AgentMux keeps its old PATH.
- [ ] Remove the harness's detect-only row from `frontend/app/store/toolchain-catalog.ts` `LOCAL_MODEL_TOOLS` (OpenCode, goose, Crush, Aider). The Toolchain pane then lists it from the registry, with its pin and drift.
- [ ] `ProviderLogo.tsx` entry (§7.10) and `DualProviderLogo.tsx` `HARNESS_NAMES`.
- [ ] `provider-brand.ts` mapping where the account vendor differs (Grok → xAI, DeepSeek Harness → DeepSeek, Muse → Meta).
- [ ] Hard-coded test lists:
  - Rust: `providers.rs` tests, `provider.rs` `ALL_KNOWN_PROVIDERS`, `agent_config.rs` filename table.
  - Frontend: `providers/index.test.ts` groups, `global-bundle-model.test.ts`, `toolchain-catalog.test.ts`.
- [ ] `check-comment-hygiene.mjs` allow-list for the harness's config paths, if comments name them.
- [ ] Live check against the installed binary (§4), recorded in the PR.
- [ ] `scripts/changeset.sh minor "…"`.

## 7. Per-harness notes

### 7.1 OpenCode

- **Two release lines.**
  - V1, `opencode-ai@1.19.0`, is what GitHub "latest", `opencode.ai/install` and the docs ship.
  - V2, `@opencode/cli@2.0.29`, is the default branch and is on npm since 2026-10-14.
  - The two differ in config shape, permission rules, server API and instructions file: V2 drops the `CLAUDE.md` fallback.
  - **Pin V1 now** and move to V2 once V2 is what upstream's installer and docs ship.
  - V2 clients share one per-user background server by default; check `--standalone` behaviour then.
- **Auth** is `cli-managed`, as with Pi: `opencode auth login` covers its many providers.
  - Credentials live in `~/.local/share/opencode/auth.json`; XDG paths are used on Windows too.
  - Verify which variable relocates them (`XDG_DATA_HOME`).
- **Startup file** is `AGENTS.md`.

### 7.2 Kilo Code CLI

- **The easiest of the nine.** npm package, native binaries, MIT, native ACP, and listed in the ACP registry.
- **Auth** is `cli-managed` (`kilo auth login`, Kilo Gateway via `KILO_API_KEY`). Data is in `<xdgData>/kilo`; `KILO_CONFIG_DIR` relocates config.
- **Telemetry is on by default** (§7.11).

### 7.3 Cline CLI

- **Install:** npm `cline`, with native builds for Windows x64 and arm64.
- **ACP and approval:** `cline --acp` is native. In ACP mode auto-approve is off and AgentMux answers permission requests. In plain CLI mode it is on.
- **Embedding hazards:**
  - It starts a background hub daemon on port 25463 that outlives the command. Check whether ACP mode starts it.
  - Telemetry is on by default.
- **Auth:** `cline auth`. Data is in `~/.cline`; `CLINE_DIR` / `CLINE_DATA_DIR` relocate it.

### 7.4 goose

- **Install:** a GitHub-release binary, not npm.
  - It follows the non-npm precedent: hint command plus `known_install_paths`, with `versionSource: github-release aaif-goose/goose`.
  - A managed installer that downloads the pinned release asset and checks its published checksum would be a later improvement. It would serve goose and Crush.
- **Set `GOOSE_DISABLE_KEYRING=1`.** goose stores secrets in the OS keyring by default, and a keyring prompt can block a process nobody is watching.
- **`GOOSE_MODE` defaults to `auto`,** which approves everything. That's fine under ACP, where AgentMux answers permission requests, but the card should say so.
- **There is no default model;** the user configures a provider first. `GOOSE_PATH_ROOT` relocates everything.
- **Prefer ACP for the long term:** upstream plans to replace the Rust CLI with an ACP-based TUI.

### 7.5 Grok Build

- **Install:** xAI's installer or winget only.
  - The binary is `grok.exe` in `%USERPROFILE%\.grok\bin` on Windows and `~/.grok/bin` elsewhere; those go in `known_install_paths`.
  - `versionSource` is `vendor`: the stable channel is published at `x.ai/cli/stable`, but there are no GitHub releases.
- **Auth:** OAuth (`grok login`, `--device-auth`), with `XAI_API_KEY` as fallback. `GROK_HOME` relocates `~/.grok`.
- **Turn off Claude config import.**
  - By default Grok reads `~/.claude/` rules and `~/.claude.json` / `.mcp.json` MCP servers, which would pull AgentMux's Claude setup into a Grok agent.
  - The launch sets `GROK_CLAUDE_MCPS_ENABLED=0`, or `[compat.claude]` off, in the agent's `GROK_HOME` config.
- **Project rules load only in trusted folders** (`--trust`), the same problem Pi had. AgentMux trusts the agent's own folder, not the repos it works in.
- **Auto-update:** set `GROK_DISABLE_AUTOUPDATER=1`, so the pin holds.

### 7.6 DeepSeek Harness

- **Prerelease only:** the `latest` npm tag is `0.2.0-rc.2`, and upstream promises breaking changes. The card says "preview".
- **It ships no TUI;** its interactive UI is a local web app, so ACP is the only fitting path.
  - Its ACP lacks `session/load`, so no history replay. Resume only reopens the session.
- **It uploads session logs to DeepSeek by default.** The launch disables that upload in the agent's profile (`session-log-deepseek` `enabled: false`) unless the owner decides otherwise (§9).
- **Auth:** `DEEPSEEK_API_KEY` (ApiKey) or browser sign-in. `DSH_HOME` relocates `~/.dsh`.
- **There is no `--model` flag;** the model is chosen over ACP.
- **Naming:** upstream's brand guidelines restrict "DeepSeek Harness" in product names. AgentMux shows it as the harness's name, which identifies it, and does not call any AgentMux feature by it.

### 7.7 Muse Code

- **Closed source:** an auto-updating launcher (`MUSE_NO_AUTO_UPDATE=1` stops the updates), and no ACP.
- **Two ways to drive it:**
  - **MSP:** `muse serve`, documented, typed, pre-1.0. Pin its schema fingerprint. This means a new controller, shaped like the ACP one.
  - **`muse exec --json`:** a subprocess translator, but its event schema is undocumented.
  - **Spike result (1.4.7-R6101.1):** MSP. `exec --json` prints Muse's internal log records, not a published contract, one process per prompt. `muse serve` is typed, reports its schema fingerprint in `initialize` (pinned in `blockcontroller/msp_protocol.rs`), takes MCP servers in `session/start`, and fails a signed-out turn with `authRequired` instead of hanging.
- **Auth:** `META_API_KEY` or browser sign-in. Credentials are in `~/.config/muse/auth.json`; `MUSE_AUTH_PATH` overrides it.
- **Terms and trust:**
  - Meta's subscription "works only through the Muse Code CLI". Spawning the CLI, which is what AgentMux does, is consistent with that. Reusing its key elsewhere is not.
  - Subscriptions default to "contributor mode", where user content trains Meta's models. The card says so.
  - Project rules load only once the workspace is trusted.

### 7.8 Crush

- **License:** FSL-1.1-MIT.
  - It allows use and redistribution except for a "Competing Use", which includes offering substantially similar functionality in a commercial product.
  - Each version becomes MIT two years after release.
  - The license grants no right to the Charm and Crush marks.
- **What AgentMux does about the license:**
  - It installs Crush only from Charm's own channel, the npm package `@charmland/crush`, so the user receives it under Charm's terms. AgentMux never bundles or ships the binary.
  - It shows a letter mark rather than Charm's artwork.
  - It asks Charm in writing before doing more.
- **No ACP and no JSON output:** `crush run` prints plain text and auto-approves its session. The upstream ACP PR is open.
  - Until it merges, Crush uses the terminal tier. When it merges, Crush moves to ACP.
- **Defaults to switch off:**
  - Usage metrics: set `CRUSH_DISABLE_METRICS=1`.
  - Crush adds an `Assisted-by` trailer to commits by default. Leave that to the user's config.

### 7.9 Aider

- **Development has slowed:** last commit 2026-05-22, last release 0.86.2 on 2026-02-12. There is an active fork, `aider-ce`.
- **Plan for upstream Aider,** in the terminal tier:
  - Launch: `aider --read CONVENTIONS.md`, with AgentMux's startup instructions written to `CONVENTIONS.md`.
  - Install: `uv tool install aider-chat==0.86.2`. Add a `systemPrereqs` entry for uv, using the existing one-click installs.
  - `versionSource`: `pypi aider-chat`.
- **No MCP:** Aider gets no AgentMux tools.
- **Embedding hazards:**
  - It writes `.aider*` files into the repo and edits `.gitignore`.
  - Analytics are enabled for a random subset of users by default; the launch passes `--analytics-disable`.
- Supersedes the Aider half of `SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02.md`.

### 7.10 Logos

- **From `@lobehub/icons-static-svg`:**
  - OpenCode: `opencode` (mono)
  - Kilo: `kilocode`
  - Cline: `cline`
  - goose: `goose`
  - Grok Build: `grok`
  - DeepSeek Harness: `deepseek-color`, the company mark, since DSH has no separate one in the package. Its own `icon.svg` is MIT in its repo.
- **Elsewhere:**
  - Aider: its official `logo.svg` (Apache-2.0 repo).
  - Muse Code: a letter mark. There's no official SVG, and Meta's generic marks don't name Muse.
  - Crush: a letter mark (§7.8).
- **Unaltered marks:** follow each vendor's brand rules. xAI's say to use logos unaltered.

### 7.11 Telemetry and data defaults

| Harness | Default | Opt-out |
|---|---|---|
| Cline | telemetry on | `telemetryOptOut: true` |
| Kilo | on | `experimental.openTelemetry: false` |
| Crush | metrics on | `CRUSH_DISABLE_METRICS=1` / `DO_NOT_TRACK=1` |
| Grok Build | likely on for consumers (undocumented) | `GROK_TELEMETRY_ENABLED=false` |
| DeepSeek Harness | session logs uploaded with requests | `session-log-deepseek` `enabled: false` |
| Muse Code | contributor mode on subscriptions | a setting in Muse |
| Aider | analytics for a random subset | `--analytics-disable` |
| goose | off by default | — |
| OpenCode | none documented | — |

**Recommendation:** AgentMux applies each documented opt-out by default, shows it on the card, and lets the user turn it back on. That is an owner decision (§9).

## 8. Plan

One PR per numbered item, each merged before the next harness that depends on it.

**Phase 0: shared work** (§5)
1. Split `providers.rs` and lower the size baseline.
2. No Claude fallbacks for ACP providers; generalise `prompt_arg_flag`.
3. `agentmux-mcp` in ACP `mcpServers`.
4. ACP token usage, per agent once measured (§5 item 4).
5. `versionSource` in the catalog.
6. The terminal tier.
7. Seed generator fixed and checked.

**Phase 1: ACP harnesses**, roughly easiest first. Each PR runs the §6 checklist and the §4 live check.

8. Kilo Code CLI
9. Cline CLI
10. OpenCode (V1)
11. goose
12. Grok Build
13. DeepSeek Harness, marked preview

**Phase 2: the rest**
14. Aider, terminal tier.
15. Crush, terminal tier. It moves to ACP when upstream ships it.
16. Muse Code: the spike (MSP vs `exec --json`), then its PR.

**Phase 3: everything outside this repo.** These run as the harnesses land, batched per phase rather than per harness.

17. **agentmux-docs**, one PR per phase that ships.
    - Rows in the harness tables:
      - `first-agent.md` "Harnesses and providers" (also the count and the startup-file note)
      - `auth.md` (per-provider summary, OAuth/API-key lists, env-var table)
      - `security/update-model.md` (pins; and the non-npm update model, which the page says doesn't exist)
      - `internals/provider-cli-integration.md`
      - `settings.md` config-dir variables
      - `internals/muxapi.md`: which harnesses get MCP tools
    - Counts and lists in `getting-started.mdx`, `index.mdx` and `user-guide.mdx`.
    - Rewrite `installation.md`'s Toolchain "Local models" section, which says OpenCode, goose, Crush and Aider can't be launched yet.
    - Egress hosts in `security/data-sovereignty.md` and `security/network-exposure.md`: pypi.org, GitHub release downloads, x.ai, dev.meta.ai.
    - Retake `agent-picker.png` and `agent-create.png`.
    - Fix the drift already there (Antigravity is listed as an npm package; Pi's package name is the old one).
    - Bump the `src/agentmux` submodule first, because the source-link check fails on paths that don't exist at the pinned commit.
18. **This repo's own docs.**
    - `README.md`: the provider lists at lines 35–62, and which tier each runs on.
    - `docs/providers/`.
19. **Version drift report.**
    - npm harnesses (OpenCode, Kilo, Cline, DeepSeek Harness, Crush) need no change: the report reads `catalog.ts`.
    - pypi, github-release and vendor sources need the report-side changes planned in the private infrastructure repo, which land after item 5.
    - Until they land, those harnesses show as unmonitored there.
20. **Website** (agentmux.ai): the harness list and "N live harnesses" counts on the home page, `llms.txt`, and the picker screenshot. Deploying it is the owner's call.
21. **Mobile app:** `providerName()` in `lib/features/feed/messages.dart` gets display names for the new ids. ACP output already renders as text there.

## 9. Owner decisions

Each has a recommendation; the plan above assumes it.

1. **OpenCode line:** V1 now, V2 when upstream's installer and docs ship it. *Recommended.*
2. **Telemetry and data upload:** AgentMux applies each harness's documented opt-out by default (§7.11), shows it on the card, and lets the user turn it back on. *Recommended.*
3. **Crush:** install from Charm's npm package only, letter mark, and write to Charm about bundling and the logo. *Recommended.*
4. **Muse Code:** terminal tier now, structured tier after the spike. *Recommended.* The alternative is to wait for ACP.
5. **DeepSeek Harness:** ship it marked "preview" while upstream is prerelease. *Recommended.* The alternative is to wait for 1.0.
6. **Aider:** upstream Aider in the terminal tier. *Recommended.* The alternatives are the `aider-ce` fork, or skipping Aider.
7. **The terminal tier itself:** a new launch kind for CLIs without a structured interface. *Recommended.* Without it, Aider and Crush can't be added.

## 10. Tests

- **Every harness PR:** the §6 test lists, plus a live check against the installed binary (§4), recorded in the PR.
- **Phase 0:**
  - item 1: no behaviour change; the full srv suite passes unchanged.
  - item 2: `pane-args-parity` no longer whitelists ACP providers.
  - item 3: an ACP agent that advertises MCP receives `agentmux-mcp` in `session/new`, and one that doesn't receives `[]`. Copilot, OpenClaw and Pi are checked live.
  - item 6: a terminal-tier harness opens a term pane in the agent's folder with its startup file written.
- **Manual, in a `task dev` build:** install, sign in and run one prompt with each harness. Harnesses that need a paid account the test machine doesn't have are marked so in their PR.

## Sources

- OpenCode: github.com/anomalyco/opencode, opencode.ai/docs, opencode.ai/v2/docs, registry.npmjs.org/opencode-ai, registry.npmjs.org/@opencode/cli
- Kilo Code: github.com/Kilo-Org/kilocode, registry.npmjs.org/@kilocode/cli
- Cline: github.com/cline/cline (apps/cli), docs.cline.bot, registry.npmjs.org/cline
- goose: github.com/aaif-goose/goose and its documentation folder, goose-docs.ai
- Grok Build: github.com/xai-org/grok-build (user guide), x.ai/build/changelog, x.ai/legal/brand-guidelines
- DeepSeek Harness: github.com/deepseek-ai/deepseek-harness, registry.npmjs.org/@deepseek-ai/dsh
- Muse Code: dev.meta.ai/docs/muse-code, dev.meta.ai/products/muse-code, meta-models.github.io/muse-code-sdk
- Crush: github.com/charmbracelet/crush (LICENSE.md, README), registry.npmjs.org/@charmland/crush
- Aider: github.com/Aider-AI/aider, aider.chat/docs, pypi.org/project/aider-chat
- ACP registry: github.com/agentclientprotocol/registry
- Icons: @lobehub/icons-static-svg 1.95.1
