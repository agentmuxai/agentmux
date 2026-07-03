# SPEC: OpenClaw integration — shared interfaces, distinct flavor

**Date:** 2026-05-17 (revised)
**Author:** AgentA (research pass)
**Status:** Draft for discussion — pre-implementation
**Discussion thread:** TBD (open one when this lands)

> Revision note: the prior draft of this file framed OpenClaw as an internal,
> OpenAI-powered Claude-Code clone built by extending the `codex` provider with
> a translator skin. That framing was wrong. OpenClaw is a real, third-party,
> open-source product from **openclaw.ai** that AgentMux already scaffolds as a
> first-class provider. This rewrite reframes the work around the user's
> "shared interfaces, distinct flavor" principle and the existing scaffold.

---

## 0. TL;DR

OpenClaw is its own product. AgentMux already has the scaffold to launch it as
a peer of Claude, Codex, Gemini, and Copilot — see `agentmux-srv/src/backend/
providers.rs:207-226` (`acpx --agent openclaw`, ACP controller, `OPENCLAW_HOME`
auth dir). The work is **not** to make OpenClaw look like Claude Code; it is
to (a) verify the scaffold actually spawns and renders, (b) route OpenClaw's
own onboarding through AgentMux's launch flow without "jamming" it, and (c)
make sure the **shared interfaces** AgentMux exposes — the agent-pane
DocumentNode model, the tool overlay, the identity/memory tabs — render
OpenClaw's events cleanly.

The OpenAI-as-backing-model question (the user's "verify OpenAI OAuth can be
used") is a **configuration matter inside OpenClaw**, not an AgentMux OAuth
flow. Section 4 dissects what that means concretely.

---

## 1. What OpenClaw actually is

From [openclaw.ai](https://openclaw.ai) and [docs.openclaw.ai](https://docs.openclaw.ai):

- **Open-source personal AI assistant.** GitHub-hosted. Apache or similar OSS
  license. Single-user, on-device — Mac, Windows, Linux.
- **Two surfaces:** a CLI (`openclaw onboard`, `openclaw tui`, `openclaw acp …`,
  `openclaw doctor`) and a daemon (`openclaw gateway`) that runs on
  `localhost:18789` and hosts sessions, agent identities, and channel routing.
- **Model-agnostic.** Supports Anthropic Claude (default), OpenAI GPT, and
  local models (e.g. MiniMax 2.5). Bring-your-own-key per backend.
- **Chat-app multiplexing.** Routes the same agent through WhatsApp, Telegram,
  Discord, Slack, Signal, iMessage in DMs and group chats.
- **Full system access** with sandboxing options — file IO, browser control,
  shell execution. Skill / plugin system.
- **Onboarding via `openclaw onboard`** — described in docs as a "guided setup
  with pairing flows." Specifics (interactive vs scripted, browser-required vs
  not) are not fully documented; see §8 open questions.
- **No central OAuth.** OpenClaw onboards into the user's own environment.
  Provider credentials (Anthropic API key, OpenAI API key, etc.) are entered
  during onboarding and stored under `~/.openclaw/`.

OpenClaw's relationship to AgentMux: **OpenClaw is one CLI agent among many
that AgentMux can host in an agent pane.** It is not an AgentMux subsystem,
not a clone target, and not something to graft Claude-Code semantics onto.

---

## 2. What AgentMux already has scaffolded

| File | Line | What it does |
|---|---|---|
| `agentmux-srv/src/backend/providers.rs` | 207-226 | `OPENCLAW` `ProviderConfig` — `cli_command: "acpx"`, `controller_type: Acp`, `launch_args: ["--agent", "openclaw"]`, `auth_config_dir_env_var: "OPENCLAW_HOME"`, `npm_package: "@openclaw/acpx"`. |
| `agentmux-srv/src/backend/blockcontroller/acp.rs` | 11 | Doc-comment lists `acpx --agent openclaw` as a canonical example of an ACP-protocol agent the controller drives. Same JSON-RPC lifecycle as `gemini --acp` and `copilot --acp`. |
| `agentmux-srv/src/identity/auth_patterns.rs` | 57, 71 | `openclaw` listed in `is_api_key_provider()` and in the empty-matcher branch of `patterns_for()`. No OAuth URL extraction — credentials come from OpenClaw's own onboarding. |
| `frontend/app/view/agent/providers/index.ts` | 127-150 | `PROVIDERS.openclaw` entry — `cliCommand: "acpx"`, `authType: "api-key"`, `authCheckCommand: ["openclaw", "doctor"]`, `authLoginCommand: ["openclaw", "onboard"]`, `controllerType: "acp"`. |
| `frontend/app/view/forge/forge-constants.ts` | 115-126 | Agent-card metadata: blurb "ACP orchestration platform" + popover describing OpenClaw as a coordinator that runs other agents. |
| `agentmux-srv/forge-seed.json` | 124-147 | Seed agent persona for `openclaw` with startup skill. |
| `docs/specs/openclaw-agent-runtime.md` | — | Earlier spec describing `openclaw tui` (interactive TUI backed by the gateway) and `openclaw acp client` (sub-agent protocol bridge) as the two integration surfaces. |
| `docs/specs/openclaw-widget.md` | — | Companion spec for embedding OpenClaw's web dashboard at `localhost:18789` as an AgentMux widget. |

**Status of the scaffold:** wired but unverified end-to-end. The
`cli_command: "acpx"` value is an early guess at the binary name; current
[docs.openclaw.ai](https://docs.openclaw.ai) references `openclaw acp` as a
first-party subcommand, not a separate `acpx` binary, and the
`@openclaw/acpx` npm package isn't visible in the documented CLI reference.
Verifying the actual binary/subcommand is Phase α work.

---

## 3. The shared-interface principle

The user's framing — *"we don't want to jam anything, each should keep their
own unique flavor, but we encourage shared interfaces"* — maps onto a small
number of seams that AgentMux already owns. The integration question for any
new provider is **which seams it plugs into vs which it routes around**.

### 3.1 The seams

| Seam | What it is | Where it lives | What it normalizes |
|---|---|---|---|
| **DocumentNode model** | The renderable tree the agent pane displays — assistant text blocks, tool-call blocks, tool-result blocks, markdown sections, thinking sections, cost banner. | `frontend/app/view/agent/types.ts`, `virtualization/state.ts`, `virtualization/renderers.ts`, `components/ToolBlock.tsx` | What appears on screen and how it's structured / collapsible / styled. |
| **StreamEvent union** | The provider-agnostic event shape translators emit. Variants: `text`, `tool_call`, `tool_result`, `thinking`, `cost`, `error`, `done`. | `frontend/app/view/agent/stream-parser.ts` + `state.ts` | What translators must produce. ACP, claude-stream-json, codex-json, gemini-json all funnel through here. |
| **Translator interface** | Per-provider parser that consumes the CLI's wire format and emits `StreamEvent`s. | `frontend/app/view/agent/providers/translator.ts` + `*-translator.ts` files | The boundary between provider-specific wire formats and the shared event model. |
| **Launch flow** | 3-phase modal: resolve binary → check/run auth → register controller. | `frontend/app/view/agent/flows/launch-flow.ts` + `components/PreLaunchAuthPanel.tsx` + `auth/auth-flow-controller.ts` | When and how the user authenticates and what the modal shows during onboarding. |
| **Auth-URL pattern matchers** | Per-provider regex pack that pulls OAuth URLs or device codes from CLI stdout. | `agentmux-srv/src/identity/auth_patterns.rs` | The seam between "CLI prints stuff" and "AgentMux opens a browser." |
| **Identity / Memory tabs** | Per-agent identity bundle dropdown + persistent memory tab inside the agent pane. | `frontend/app/view/agent/` settings panel; `db_identities`, `db_memories` schema. | Multi-account hygiene and conversation persistence — provider-independent. |
| **Bash-stream overlay** | `agentmux-bashwrap` hook + the streaming output viewer that shows partial stdout while a tool runs. | `.claude/settings.json` PreToolUse hook → `agentmux-bashwrap` crate; `BashOutputViewer.tsx`. | Mid-tool-execution streaming UX — only fires if the provider routes its shell tool through a hookable boundary. |
| **Container path** | Per-agent containerized run inside `agentmux/<provider>:latest`. | `cli-catalog.ts` `containerSupported` flag + container build configs. | Sandboxed execution (defer for openclaw v1). |

### 3.2 Provider matrix — what plugs into what

| Provider | DocumentNode | StreamEvent | Translator | Launch flow | Auth-URL matcher | Identity/Memory | Bash overlay | Container |
|---|---|---|---|---|---|---|---|---|
| **Claude** | shared | shared | `claude-translator.ts` (stream-json) | shared | shared (`match_claude_url`) | shared | **yes** — `agentmux-bashwrap` via `.claude/settings.json` PreToolUse | yes |
| **Codex** | shared | shared | `codex-translator.ts` (codex-json) | shared | shared (`match_codex_url`) | shared | no — Codex's internal tool dispatch has no hook surface | yes |
| **Gemini** | shared | shared | `gemini-translator.ts` (stream-json) | shared | shared (`match_gemini_url`) | shared | no — same reason as Codex | yes |
| **Copilot** | shared | shared | ACP translator (JSON-RPC 2.0) | shared | shared (`match_copilot_device_code`) | shared | no | no |
| **OpenClaw** *(this spec)* | shared | shared | ACP translator | shared *(but defers to openclaw onboard for credentials)* | none — `is_api_key_provider` | shared | **defer** — needs investigation (§7) | no (v1) |
| **Kimi** | shared | shared | `kimi-translator.ts` (stream-json) | shared | none — `is_api_key_provider` | shared | no | yes |
| **Pi** | shared | shared | ACP translator | shared | none — `is_api_key_provider` | shared | no | no |

The horizontal sameness is the point. OpenClaw "keeps its own flavor" by (a)
running its own binary, (b) using its own onboarding flow, (c) speaking ACP
on its own terms, (d) routing through its own gateway when active. AgentMux
"shares interfaces" by giving OpenClaw the same agent-pane render path, the
same identity/memory tabs, and the same launch modal shell every other
provider gets.

What is **explicitly not** shared and shouldn't be forced:

- OpenClaw's onboarding is interactive and conversational — that flavor stays.
  AgentMux does not impersonate it with a different UI.
- OpenClaw's model selection is internal (config + onboard wizard). AgentMux
  does not expose a `--model` dropdown for openclaw the way it does for Claude.
- OpenClaw's gateway-multi-channel-routing (WhatsApp / Telegram / Discord)
  has no analog in AgentMux's agent pane; we don't try to render it.

---

## 4. The OpenAI-as-backing-model question

**Confirmed via openclaw.ai docs + community guides (2026-05-17 research):**
OpenClaw ships a **bundled `codex-harness` plugin** that runs embedded OpenAI
agent turns through Codex's own app-server, in place of OpenClaw's default
Pi harness. **ChatGPT-subscription OAuth credentials work as the auth
source** — the OAuth profile that Codex CLI writes during `codex login` is
the same shape OpenClaw's `openai-codex:*` auth profile expects.

### 4.1 How OpenClaw uses Codex as its brain

Three OpenClaw-side ingredients (verified from `docs.openclaw.ai/plugins/codex-harness`):

1. **Plugin enabled:** `plugins.entries.codex.enabled = true` in OpenClaw's
   config. The `codex-harness` plugin ships bundled — no separate install.
2. **Model selected:** `agents.defaults.model = openai/gpt-5.5` (or any
   `openai/gpt-*` ref). The `openai/` prefix routes the agent's turn through
   the Codex harness instead of Pi.
3. **Auth profile registered:** an `openai-codex:*` profile under
   `auth.profiles`, with `auth.order.openai` listing it first (or a
   subscription-first / API-key-fallback ordering).

The OpenClaw-native command to enroll a ChatGPT-subscription identity is:

```
openclaw models auth login --provider openai-codex
```

This walks the user through the same "Sign in with ChatGPT" OAuth flow
that Codex CLI's `codex login` does, then writes the resulting credentials
into OpenClaw's auth store. **When OpenClaw subsequently launches the
Codex app-server as the agent's brain, it strips `CODEX_API_KEY` and
`OPENAI_API_KEY` from the spawned child's env** so the OAuth profile takes
precedence — no risk of an ambient API key silently overriding the
subscription auth.

### 4.2 Auth resolution order (for the local stdio Codex launch)

When OpenClaw spawns the Codex app-server to handle an `openai/*` turn,
auth is selected in this order:

1. Ordered OpenAI auth profiles (`auth.order.openai`) — subscription
   profile first if present, API-key profile second.
2. The app-server's existing account in that agent's per-identity Codex
   home directory (lets a previously-authed `codex login` in the agent's
   sandbox survive).
3. Env-var fallback (`CODEX_API_KEY` → `OPENAI_API_KEY`) — only used if 1
   and 2 are absent.

The user's question — "can the codex brain be gotten for openclaw's use
via the openai oauth auth?" — answer: **yes, via `openclaw models auth
login --provider openai-codex` which performs the ChatGPT-subscription
OAuth flow and writes a profile OpenClaw uses as the Codex harness's auth
source.** No API key needed.

### 4.3 What's split between OpenClaw and Codex when this is configured

| Layer | Owner |
|---|---|
| Routing, channels, sessions, cron jobs, memory files | **OpenClaw** |
| Persistent agent identity, dynamic-tool layer, approvals | **OpenClaw** |
| Visible transcript / channel mirror | **OpenClaw** |
| Low-level agent session (turn, thread resume) | **Codex app-server** |
| Native tool continuation, compaction | **Codex app-server** |
| Model API call, OAuth token refresh | **Codex app-server** |

So even with the Codex harness enabled, OpenClaw still owns the user-facing
agent surface — the brain is Codex but the agent is still OpenClaw.

### 4.4 What AgentMux should do

For Phase β (§6 below): surface OpenClaw's model selection as **informational
only** in the launch panel — a read-only "configured model: openai/gpt-5.5"
pill sourced from `openclaw doctor` (or whatever subcommand exposes that
config in the bundled version). No dropdown that mutates the model — that
would jam OpenClaw's config.

If we later want a "switch backing model" UX, the right pattern is a
**launch into `openclaw models auth login --provider <provider>`** action
that hands control back to OpenClaw's own enrollment wizard. AgentMux's
launch flow already supports deferring to a CLI's own subcommand (that's
what `authLoginCommand` does for Kimi — runs `kimi login` inline in the
pane). Re-use that mechanism rather than reimplementing OpenClaw's auth
wizard inside the launch modal.

**Sources for §4** (added 2026-05-17 after the user's clarification):

- [Codex harness — docs.openclaw.ai](https://docs.openclaw.ai/plugins/codex-harness)
- [openai.md — github.com/openclaw/openclaw](https://github.com/openclaw/openclaw/blob/main/docs/providers/openai.md)
- [model-providers.md — github.com/openclaw/openclaw](https://github.com/openclaw/openclaw/blob/main/docs/concepts/model-providers.md)
- ["I Switched My Agent Stack from Claude to OpenAI Codex" — mager.co](https://www.mager.co/blog/2026-04-11-openclaw-openai-codex/) (community write-up of the exact path above)

---

## 5. The hardcoded-to-Claude runner in `agents/runner.rs`

The prior draft flagged this and it's still real. `agentmux-srv/src/agents/
runner.rs:82-117` hard-codes `claude --print --output-format=stream-json` and
the Claude translator. The runner is the **Drone Agent-block** path —
one-shot, headless agent spawns from inside a Drone.

**Impact on OpenClaw:** the interactive agent-pane path goes through
`blockcontroller/acp.rs`, not `runner.rs`. OpenClaw users launching from the
agent picker hit the ACP controller; that path is unaffected by the runner
hardcoding. The hardcoding only blocks workflow Agent blocks from running
OpenClaw as the executor — a real limitation but not a v1 blocker for
"OpenClaw works in the agent pane."

Keep this as a Phase ε cleanup, scoped under the unified-agent-types Phase B
work, not as a prerequisite for shipping OpenClaw to the agent picker.

---

## 6. Integration plan

### Phase α — verify the scaffold (1-2 PRs)

**Goal:** OpenClaw spawns from the agent picker, ACP handshake completes,
output renders in the agent pane.

1. **Verify the binary.** Confirm whether `acpx` or `openclaw acp client`
   (or `openclaw acp serve`) is the canonical AgentMux launch invocation in
   today's openclaw release. Update `agentmux-srv/src/backend/providers.rs`
   lines 210, 212, 222 if the answer is `openclaw acp …` instead of `acpx`.
   Mirror in `frontend/app/view/agent/providers/index.ts` lines 130-149.

2. **Verify ACP handshake.** Spawn `openclaw acp …` (or `acpx`) by hand from
   a shell, send the `initialize` / `initialized` / `session/create` /
   `session/prompt` sequence `blockcontroller/acp.rs:480-502` will send, and
   confirm OpenClaw responds correctly. Document any field-shape differences
   (e.g. does OpenClaw expect `cwd` in `session/create` params? does it
   support our `workspaceRoots` array?).

3. **Verify output renders.** Drive a one-turn prompt and confirm the
   agent-pane DocumentNode renderer accepts OpenClaw's `session/update`
   notifications without translator changes. If OpenClaw emits an ACP variant
   that doesn't map cleanly, scope a small OpenClaw-flavored ACP translator
   in `frontend/app/view/agent/providers/`.

4. **Forge card copy.** Update `frontend/app/view/forge/forge-constants.ts`
   lines 116-126 — the "ACP orchestration platform" blurb is technically
   accurate but undersells the product. Suggested:
   - blurb: `"Open-source personal AI assistant"`
   - popover: brief mention that OpenClaw is model-agnostic, runs locally,
     and onboards via its own `openclaw onboard` wizard. Encourage users to
     run onboarding before launching from AgentMux.

**Exit criteria:** user clicks the OpenClaw card → AgentMux runs
`openclaw doctor` to verify install + onboard state → if not onboarded,
modal explains "run `openclaw onboard` first" with a one-click "open
terminal here" affordance → otherwise spawn → ACP session live → user types
prompt → response streams in the agent pane.

### Phase β — onboarding UX inside the launch flow (1 PR)

**Goal:** first-time OpenClaw users have a clear path through `openclaw
onboard` without leaving AgentMux.

Open question §8.2 gates the implementation choice:

- **(β.A) Defer to OpenClaw's own wizard.** Launch panel detects "not
  onboarded" via `openclaw doctor` exit code, surfaces a single CTA that
  spawns `openclaw onboard` in a regular Terminal pane (not the Agent pane,
  because onboarding may be interactive in ways that don't fit the
  StreamEvent model). When the user is done, they re-click the OpenClaw
  agent card and AgentMux retries `openclaw doctor`.

- **(β.B) Embed the wizard.** Spawn `openclaw onboard` inside the launch
  modal itself, capture stdout/stdin, render its interactive prompts. Larger
  surface, fragile if openclaw changes the wizard shape, but slicker UX.

**Recommendation:** β.A. It respects OpenClaw's flavor (its onboarding is its
own product surface), it's smaller to ship, and it generalizes to any
future provider whose login flow doesn't fit a streaming event model. Track
β.B as a "polish if users complain" follow-up.

`authLoginCommand: ["openclaw", "onboard"]` is already wired in
`providers/index.ts` line 137; the launch flow already knows how to run a
provider's login command. Phase β work is mostly UX copy — explain to the
user that OpenClaw's onboarding is its own product surface and they're
about to leave the launch modal briefly.

### Phase γ — model selection inside OpenClaw (1 PR — informational only)

**Goal:** user can see which model OpenClaw is configured to use, without
AgentMux mutating that config.

Scope:

- Parse `openclaw doctor` output (or whatever subcommand reports current
  config — `openclaw config show`?) to extract the currently-selected
  backing model. **Open question §8.3** — what command and what shape?
- Render that as a read-only pill in the agent pane's settings panel:
  `Backing model: gpt-4o (via OpenClaw)`. Click → opens
  `https://docs.openclaw.ai` configuration section in browser.
- "Reconfigure backing model" action button — runs `openclaw onboard
  --reconfigure` in a Terminal pane (same pattern as β.A).

**Non-goal:** AgentMux does not provide a model dropdown for OpenClaw. Each
provider keeps its own flavor of model-selection UX. Claude has a flag-driven
dropdown because Claude's CLI supports `--model`; OpenClaw has an internal
config because that's OpenClaw's idiom.

### Phase δ — identity / memory persistence (1 PR)

**Goal:** OpenClaw participates in the shared identity-bundle and memory-tab
plumbing without leaking state across AgentMux identities.

Scope:

- Verify `OPENCLAW_HOME` redirect actually points `~/.openclaw/` somewhere
  inside `{dataDir}/auth/openclaw/<bundle>/` per
  `docs/specs/provider-auth-isolation.md`. Test: switch identity bundles in
  the launcher, confirm each bundle has its own `~/.openclaw/`-shaped tree.
- Confirm the agent-pane Memory tab (cog → settings → Memory) reads/writes
  to a `db_memories` row keyed by OpenClaw's identity bundle. The seed row
  in `forge-seed.json:124-147` (`id: openclaw`) is the migration target.
- Named-agent continuation: typing a previously-used OpenClaw agent name
  re-uses the prior working directory. ACP `session_id` resumption is not
  needed — `blockcontroller/acp.rs` re-initializes a fresh session per pane
  open, and OpenClaw's gateway persists conversation state independently.

**Exit criteria:** two AgentMux identity bundles each onboarded into
separate `~/.openclaw/`-shaped trees, conversations persist across pane
close/reopen, no credential leakage across bundles.

### Phase ε — `agentmux-bashwrap` integration (deferred — needs empirical data)

**Goal:** match Claude's mid-tool-execution streaming bash output UX.

OpenClaw executes shell commands as part of its skills. Whether
`agentmux-bashwrap` can intercept those depends on:

- Does OpenClaw spawn shell commands via a hookable boundary (PreToolUse
  hook, MCP hook, plugin)?
- Does the spawn inherit the AgentMux-injected `PATH` so a bashwrap shim
  could intercept generically?

Both are unknown from the public docs. Defer until Phase α produces
empirical data on what OpenClaw's shell-tool flow looks like in
`session/update` notifications. If shell calls arrive as opaque "ran X,
output Y" pairs (no mid-execution stream), the v1 UX is "tool block shows
command + final result, no streaming partial" — matching the current Codex
and Gemini UX.

### Phase ζ — workflow Agent block support (deferred)

Decouple `agents/runner.rs` from `claude` so OpenClaw (and Codex, Gemini)
can run as a workflow Agent block executor. Tracked under Phase B
unified-runner work, not a v1 OpenClaw blocker.

---

## 7. Files that change (by phase)

| Phase | File | Action |
|---|---|---|
| α | `agentmux-srv/src/backend/providers.rs` | Verify + correct `cli_command`, `npm_package`, `launch_args` for openclaw if `acpx` is not the right value. |
| α | `frontend/app/view/agent/providers/index.ts` | Mirror the providers.rs correction. |
| α | `frontend/app/view/forge/forge-constants.ts` | Rewrite OpenClaw blurb + popover (lines 115-126). |
| α | `agentmux-srv/forge-seed.json` | Refresh OpenClaw seed description (line 130) to match new blurb. |
| α | `frontend/app/view/agent/providers/openclaw-translator.ts` | NEW *(only if OpenClaw's ACP variant differs from Gemini/Copilot ACP enough to need provider-specific shaping)*. |
| β | `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx` | "Onboard with OpenClaw" copy + handoff CTA for the `openclaw onboard` flow (β.A path). |
| β | `frontend/app/view/agent/auth/auth-state.ts` | New `awaiting_openclaw_onboard` sub-state if β.A's "open a Terminal pane and poll doctor" requires explicit FSM step. |
| γ | `frontend/app/view/agent/agent-view.tsx` (settings panel) | Read-only "backing model" pill + "reconfigure" action. |
| δ | (no new files — exercises existing identity/memory plumbing) | |

Estimated **+1 file (translator, contingent)**, **5 modified**, **~300 LoC**
for Phases α-δ.

---

## 8. Open questions

1. ~~**Does OpenClaw natively support OpenAI ChatGPT-subscription OAuth?**~~
   **Resolved 2026-05-17 (see §4):** Yes, via the bundled `codex-harness`
   plugin. `openclaw models auth login --provider openai-codex` performs
   the ChatGPT-subscription OAuth flow and writes an `openai-codex:*`
   profile under `auth.profiles`. With `plugins.entries.codex.enabled =
   true` + `agents.defaults.model = openai/gpt-*` + a subscription-first
   `auth.order.openai` ordering, OpenClaw spawns Codex's app-server as the
   agent's brain using that OAuth profile. `CODEX_API_KEY` / `OPENAI_API_KEY`
   are stripped from the spawned child's env so the OAuth takes precedence.
   No API key required.

2. **β.A vs β.B for the onboarding UX.** Defer to OpenClaw's own wizard (β.A)
   keeps the flavor distinct; embedding it (β.B) is slicker but jammier.
   Recommend β.A — confirm with user.

3. **What command reports OpenClaw's current backing model?** `openclaw
   doctor`? `openclaw config show`? `openclaw status --json`? Phase γ
   depends on this; needs Phase α exploratory work to find out.

4. **`acpx` vs `openclaw acp …`.** The scaffolded `cli_command: "acpx"` in
   `providers.rs:210` doesn't match what current `docs.openclaw.ai` documents.
   Phase α verifies which one is right. If both exist, prefer the
   first-party `openclaw acp` subcommand for reliability.

5. **Does OpenClaw's ACP emit tool calls in a shape our DocumentNode renderer
   already handles?** ACP is a protocol; the *content* of `session/update`
   notifications varies by agent. The Gemini and Copilot ACP paths might have
   already normalized this; Phase α confirms whether OpenClaw fits the same
   shape or needs a small `openclaw-translator.ts`.

6. **`agentmux-bashwrap` applicability** (Phase ε). Does OpenClaw expose a
   PreToolUse-style hook surface, or is its shell execution opaque to
   external interception? Defer until empirical data exists.

7. **Container path.** Currently `containerSupported: false` for openclaw.
   Containerizing OpenClaw is complex — its gateway, channel integrations,
   and `~/.openclaw/` state would all need to map into the container. Defer
   beyond v1 unless a specific use case drives it.

8. **Gateway lifecycle.** OpenClaw runs a daemon (`openclaw gateway`) that
   AgentMux currently doesn't manage. Does AgentMux start the gateway on
   first launch? Detect-and-warn-if-not-running? The earlier
   `openclaw-agent-runtime.md` spec assumed the user runs `openclaw gateway
   --detach` themselves. Confirm that's still the right model — or whether
   `openclaw acp` runs without needing the gateway up.

9. **Skills / plugin discovery.** OpenClaw's skill system is rich; should
   AgentMux surface installed skills anywhere (status pane, settings tab),
   or is that purely OpenClaw's own UI concern? Recommend: out of scope for
   v1. Each provider's "what tools / skills are available" UX stays inside
   the provider's own surface.

---

## 9. Verification plan

After Phase α:

- [ ] `openclaw doctor` exits 0 on the test machine — confirm install + onboard prior to launching from AgentMux.
- [ ] Click the OpenClaw agent card → ACP controller spawns the process → `session/create` succeeds → first prompt renders streaming text in the agent pane.
- [ ] Close pane, reopen → fresh ACP session; OpenClaw gateway state persists user's conversation if the user opted into that during onboard.
- [ ] `muxlog srv` shows the openclaw spawn with `OPENCLAW_HOME` env pointing inside `{dataDir}/auth/openclaw/`.
- [ ] OpenClaw and Claude can run side-by-side in two agent panes without credential collision (Claude reads `{dataDir}/auth/claude/`, OpenClaw reads `{dataDir}/auth/openclaw/`).

After Phase β:

- [ ] Fresh install, no prior onboard: clicking OpenClaw surfaces a clear "onboard first" CTA, the CTA opens a Terminal pane with `openclaw onboard` running, finishing the wizard returns the user to AgentMux, retry succeeds.

After Phase γ:

- [ ] The OpenClaw agent pane shows the currently-configured backing model.
- [ ] Changing the backing model via OpenClaw's own reconfigure flow is reflected in the pill on next launch.

After Phase δ:

- [ ] Switching identity bundles isolates OpenClaw conversations + credentials.
- [ ] Memory tab persists per-bundle.

---

## 10. Non-goals (v1)

- AgentMux brokering Anthropic / OpenAI / local-model credentials on
  OpenClaw's behalf. OpenClaw onboards into the user's environment.
- A model dropdown for OpenClaw inside AgentMux. Model selection stays in
  OpenClaw's own config.
- Embedding `openclaw onboard` inside the launch modal (β.B). Defer until
  empirical UX feedback demands it.
- Replacing OpenClaw's TUI or web dashboard. Those are OpenClaw's own
  surfaces; AgentMux composes alongside them, not on top of them.
- `agentmux-bashwrap` integration for OpenClaw shell tools. Phase ε,
  needs empirical data first.
- Containerized OpenClaw. Phase ζ at earliest.
- Workflow-Agent-block execution via OpenClaw. Phase ζ, gated on
  `agents/runner.rs` decoupling from Claude.

---

## References

- External: [openclaw.ai](https://openclaw.ai) — landing page
- External: [docs.openclaw.ai](https://docs.openclaw.ai) — CLI reference
- `agentmux-srv/src/backend/providers.rs` — provider registry (OPENCLAW at 207-226)
- `agentmux-srv/src/backend/blockcontroller/acp.rs` — ACP controller (mentions `acpx --agent openclaw` at line 11)
- `agentmux-srv/src/identity/auth_patterns.rs` — `is_api_key_provider` list (line 57), `patterns_for` dispatch (line 62-74)
- `agentmux-srv/src/agents/runner.rs` — workflow Agent block runner, hardcoded to `claude` (lines 82-117). Phase ε decoupling target, not v1 blocker.
- `frontend/app/view/agent/providers/index.ts` — frontend provider metadata (openclaw at 127-150)
- `frontend/app/view/agent/providers/translator.ts` + `*-translator.ts` — translator interface and per-provider implementations
- `frontend/app/view/agent/stream-parser.ts` — StreamEvent model
- `frontend/app/view/agent/virtualization/state.ts` + `renderers.ts` + `DocumentRow.tsx` — DocumentNode rendering
- `frontend/app/view/agent/components/ToolBlock.tsx` — tool overlay
- `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx` — launch modal
- `frontend/app/view/agent/flows/launch-flow.ts` — 3-phase launch flow
- `frontend/app/view/forge/forge-constants.ts` — agent-card metadata (openclaw at 115-126)
- `agentmux-srv/forge-seed.json` — seed personas (openclaw at 124-147)
- `docs/specs/openclaw-agent-runtime.md` — earlier `openclaw tui` integration spec; useful gateway-lifecycle context
- `docs/specs/openclaw-widget.md` — openclaw web dashboard widget spec (companion, not in scope here)
- `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` — launch flow + auth pattern reference
- `docs/specs/provider-auth-isolation.md` — per-provider `{dataDir}/auth/<provider>/` model
- `docs/specs/SPEC_ACP_CONTROLLER_2026_04_16.md` — ACP controller design
