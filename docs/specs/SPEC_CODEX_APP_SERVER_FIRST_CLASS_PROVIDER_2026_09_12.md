# Codex App Server as a First-Class AgentMux Provider

**Date:** 2026-09-12
**Status:** proposed — research and design complete; no implementation has shipped.
**Scope:** Replace Codex's one-process-per-turn `exec --json` path with a
persistent, bidirectional `codex app-server` controller; finish the interaction,
authentication, configuration, and lifecycle surfaces required for Codex to be a
first-class AgentMux agent.
**Companions:**
`SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md`,
`SPEC_CODEX_JSONL_CONTRACT_2026_08_08.md`, and
`SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`.

---

## 0. Decision

AgentMux will integrate Codex's native **App Server** over its local stdio
transport. One App Server process belongs to one live Codex agent block. AgentMux
will use App Server threads and turns instead of spawning `codex exec --json` once
per turn.

The initial production transport is:

```text
agentmux-srv
    -> codex app-server --listen stdio://
    <- newline-delimited JSON-RPC messages
```

The WebSocket transport is not part of this design. OpenAI currently documents it
as experimental and unsupported, while stdio is the default local transport.

The existing `codex exec --json` controller remains as a guarded compatibility
fallback during rollout. It is not the target architecture.

This spec replaces only the Codex controller/transport decisions in
`SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` §§4, 6, 12, 15 Slice B, and the
§18 deferral of App Server. The older spec's account binding, native
materialization, Docker projection, and ownership requirements remain in force.

---

## 1. Why this changed

The August integration deliberately chose `codex exec --json` and deferred App
Server adoption. That was a reasonable first slice: it established basic launch,
event rendering, failure handling, and `exec resume <thread_id>` continuity with a
small surface.

It is no longer the right endpoint for a rich desktop host. OpenAI's current
documentation describes App Server as the interface for embedding Codex into a
product and specifically assigns it authentication, conversation history,
approvals, and streamed agent events. Its stable-shape surface includes:

- thread start, resume, fork, read, list, archive, and compaction;
- turn start, mid-turn steer, and interrupt;
- streamed item lifecycle and text deltas;
- command, file-change, permission, MCP, and user-input requests;
- account login, logout, status, and rate-limit events;
- version-specific TypeScript and JSON Schema generation.

Those are the capabilities AgentMux has had to construct around Claude's stream
protocol. Continuing to extend the lossy `exec --json` adapter would duplicate an
upstream control plane that Codex already exposes.

### 1.1 Verified local baseline

On 2026-09-12:

- AgentMux pins `@openai/codex@0.154.0` in the frontend and Rust registries.
- `npx -y @openai/codex@0.154.0 app-server --help` succeeds.
- The pinned CLI exposes stdio, Unix-socket, and WebSocket transports plus
  `generate-ts` and `generate-json-schema` commands.
- The locally installed `codex-cli 0.154.0` exposes the same command family.
- The CLI labels App Server and schema generation experimental. Therefore this
  integration requires an explicit pin, captured schemas, contract fixtures, and
  a fallback until the compatibility gate proves the chosen release.

---

## 2. Provider landscape: OpenAI and Anthropic

### 2.1 Codex App Server

Codex App Server is a local process controlled by a rich client. It speaks a
bidirectional JSON-RPC-like protocol over stdio by default. It owns Codex thread
state and exposes structured requests and notifications to the host.

For AgentMux, this is a transport/controller replacement. The workspace remains
local, AgentMux still owns its panes and durable application state, and the user's
selected `CODEX_HOME` still defines the Codex account/configuration boundary.

### 2.2 Claude Agent SDK

Anthropic's closest local embedding product is the **Claude Agent SDK**, described
as Claude Code's tools, agent loop, and context management packaged as Python and
TypeScript libraries. It supports streaming input/output, approvals and user
input, sessions and forks, hooks, subagents, MCP, skills, plugins, checkpointing,
and usage telemetry.

It is not a language-neutral local server protocol:

- the supported library hosts are Python and TypeScript;
- the application embeds or wraps the SDK and owns deployment;
- Anthropic explicitly recommends the CLI subprocess for other languages;
- unless separately approved, third-party products may not offer `claude.ai`
  login or subscription rate limits through Agent SDK integrations and must use
  API-key authentication.

AgentMux's current Rust + Claude CLI persistent controller already supplies a
local, subscription-compatible integration. Replacing it with a Node or Python
SDK bridge would add a process and change the authentication/product contract
without closing a demonstrated user gap. This spec therefore does not migrate
Claude to the Agent SDK.

### 2.3 Claude Managed Agents

Anthropic's newest product-level embedding surface is **Claude Managed Agents**,
launched in public beta on 2026-04-09. It provides versioned agent definitions,
environments, stateful sessions, persisted event history, streamed deltas,
interrupt/redirect events, permission confirmations, tools, MCP, skills, memory,
multi-agent threads, webhooks, scheduled runs, credential vaults, and a Console
session viewer.

Managed Agents is closer to Codex App Server in capability shape, but not in
deployment or trust boundary:

| Property | Codex App Server | Claude Agent SDK | Claude Managed Agents |
|---|---|---|---|
| Control plane | Local child process | In the host application | Anthropic service |
| Tool execution | Local Codex runtime | Host-owned | Anthropic cloud or self-hosted worker |
| Conversation state | Local Codex store | Host/SDK-owned | Persisted by Anthropic |
| Primary protocol | Local JSON-RPC over stdio | Python/TypeScript API | REST + SSE events |
| Consumer subscription login | Supported by App Server | Restricted for third parties | API account |
| Best AgentMux role | Local Codex pane | Possible Claude bridge | Separate remote runtime |

Even with a Managed Agents self-hosted sandbox, orchestration remains in
Anthropic's control plane and tool inputs/results flow through it. It must not be
presented as a local-only substitute for the existing Claude controller.

### 2.4 Public roadmap check

No official Anthropic documentation, release note, or announcement found as of
2026-09-12 commits to a Claude equivalent of a local, language-neutral App Server.
This is an absence of a public commitment, not evidence that Anthropic has no
internal plan.

The observable public direction is:

1. Continue expanding the self-hosted Claude Agent SDK as the local library
   surface.
2. Continue expanding Claude Managed Agents as the hosted control plane.
3. Move execution closer to customers through self-hosted sandbox workers while
   retaining Anthropic-hosted orchestration.
4. Add richer human control to Managed Agents. On 2026-09-10 Anthropic added
   server-evaluated `auto` permission policies and `ant beta:sessions connect`,
   which can follow, steer, interrupt, and approve work in a live session.

The inference for AgentMux is that Anthropic is converging on rich-client
capabilities through Managed Agents rather than announcing a local App Server.
AgentMux should watch release notes but should not delay Codex App Server work in
anticipation of an unannounced Claude protocol.

---

## 3. Goals

1. Make Codex a persistent, bidirectional, first-class AgentMux agent.
2. Render authoritative text, reasoning, commands, file changes, MCP calls,
   plans, usage, errors, and lifecycle transitions.
3. Support command/file/network approvals and Codex user-input requests without
   defaulting every session to bypass mode.
4. Support interrupt and mid-turn steering.
5. Resume and fork Codex threads through native App Server operations.
6. Use App Server's account surface for reliable isolated Codex login and status.
7. Preserve the exact Armory account selected for the agent through
   `CODEX_HOME`.
8. Keep protocol compatibility tied to the pinned CLI using generated schemas
   and captured fixtures.
9. Preserve raw upstream frames for debugging without leaking credentials.
10. Fail closed and visibly when the protocol, account, or process is unhealthy.

## 4. Non-goals

- Migrating Claude from its working persistent CLI controller.
- Adopting Claude Managed Agents as the default Claude runtime.
- Sharing one App Server process among unrelated AgentMux accounts.
- Exposing App Server's WebSocket listener to the LAN or Internet.
- Enabling every experimental App Server method in the first release.
- Removing `exec --json` before the rollback window closes.
- Replacing AgentMux's workspace, pane, bundle, identity, or event stores with
  Codex-owned equivalents.
- Treating Codex's local thread store as the sole AgentMux transcript record.

---

## 5. Ownership and process topology

```text
AgentMux frontend
    | typed pane commands / decisions
    v
agentmux-srv CodexAppServerController
    | stdin request/response/notification frames
    v
codex app-server --listen stdio://
    | CODEX_HOME = selected isolated Armory account home
    | cwd        = agent workspace
    v
Codex thread + turn + local tools
```

### 5.1 Process ownership

- One live Codex block owns one `codex app-server` child.
- The controller owns stdin writes, stdout framing, stderr capture, request IDs,
  pending server requests, timeouts, and child shutdown.
- A child is never shared across different `CODEX_HOME` values.
- A child crash does not destroy the thread. A replacement process may call
  `thread/resume` with the persisted thread ID after a successful handshake.
- The first implementation does not use App Server daemon/proxy pooling.

### 5.2 State machine

```text
Stopped
  -> Starting
  -> Initializing
  -> Idle
  -> Running
       -> WaitingForApproval -> Running
       -> WaitingForInput    -> Running
       -> Interrupting       -> Idle
       -> Idle
  -> Recovering -> Initializing
  -> Failed
  -> Stopping -> Stopped
```

Every transition is reducer-owned. A pane must never infer `Working` only from
the presence of recent output.

---

## 6. Registry and controller model

Add a provider-neutral controller variant and a typed protocol discriminator:

```text
ControllerType::AppServer
AppServerKind::Codex
```

The Rust and frontend provider definitions declare:

```text
controller_type        = app_server
app_server_kind        = codex
app_server_args        = ["app-server", "--listen", "stdio://"]
auth_config_dir_env    = CODEX_HOME
session_id_field       = threadId
fallback_controller    = subprocess
fallback_output_format = codex-json
```

Do not make `ControllerType::Codex`. Process lifecycle is generic; protocol
semantics belong to the `AppServerKind` adapter.

The registry remains the authority for the pinned CLI, executable, auth-home
environment variable, launch command, and fallback. Frontend code must not
reconstruct those values independently.

---

## 7. Protocol lifecycle

### 7.1 Handshake

After spawning the child:

1. Start stdout and stderr readers before the first write.
2. Send exactly one `initialize` request with AgentMux client metadata.
3. Wait for a successful response within the initialization timeout.
4. Send the `initialized` notification.
5. Reject or queue pane operations until initialization completes.

Suggested client metadata:

```json
{
  "name": "agentmux",
  "title": "AgentMux",
  "version": "<AgentMux version>"
}
```

The initial production capability set excludes `experimentalApi`. Experimental
features require one-by-one evidence, UI behavior, and contract tests.

### 7.2 Thread start and resume

- With no stored thread ID, call `thread/start` using the effective model, cwd,
  and approved configuration.
- With a stored thread ID, call `thread/resume` before accepting a new turn.
- Persist the returned thread ID in block metadata before reporting the pane
  ready.
- If resume reports a genuinely missing or corrupt thread, surface a typed
  recovery choice. Do not silently start a new conversation under the old pane.
- Quick Fork uses `thread/fork`; it never copies a thread ID and pretends the
  result is independent.

### 7.3 Turns

- User messages call `turn/start` against the loaded thread.
- Follow-up input during an active turn calls `turn/steer` only when the UI
  explicitly indicates steering; ordinary queued messages remain queued.
- Stop calls `turn/interrupt` and waits for the terminal turn notification.
- Manual compact calls `thread/compact/start` and renders the resulting
  `contextCompaction` item lifecycle.

### 7.4 Request correlation

The controller maintains two disjoint maps:

- client request ID -> pending AgentMux operation;
- server request ID -> pending approval/input UI request.

IDs are opaque JSON values at the transport boundary and normalized into an
internal typed key. Duplicate responses, unknown IDs, and responses received
after cancellation are logged and ignored; they never resolve an unrelated
request.

---

## 8. Event and item mapping

Create an App Server protocol adapter rather than feeding JSON-RPC envelopes into
the existing `CodexTranslator` unchanged.

| App Server input | AgentMux representation |
|---|---|
| `turn/started` | controller phase `Running` |
| `item/started` | typed document/tool item start |
| `item/agentMessage/delta` | transient text preview |
| reasoning deltas/items | thinking/reasoning block |
| command execution item | `Shell` tool call, chunks, result, exit code |
| file change item | structured diff/file-change tool |
| MCP tool item | provider-qualified MCP tool call/result |
| plan item/update | plan/todo surface |
| `item/completed` | authoritative item reconciliation |
| `turn/completed` | terminal status, usage, controller `Idle` |
| `thread/status/changed` | thread health/activity metadata |
| process EOF before terminal event | typed provider crash |

Delta events are previews. Completed items are authoritative. The reducer must
reconcile them by upstream item ID so text or tool output is never duplicated.

Unknown notifications are retained in the raw event log and ignored by the
renderer. Unknown server-initiated requests fail closed with a protocol error;
they must never be auto-approved.

---

## 9. Approvals and user input

App Server may initiate requests for:

- command execution approval;
- file-change approval;
- network or broader permission grants;
- `tool/requestUserInput`;
- MCP elicitation;
- experimental dynamic tools, if enabled in a later slice.

### 9.1 Required behavior

1. Scope every prompt by block, thread, turn, request, and item ID.
2. Reuse AgentMux's existing decision/question presentation primitives where
   their semantics match.
3. Render command, cwd, reason, requested roots/network target, and available
   decisions without lossy string flattening.
4. Send only a decision explicitly selected by the user or allowed by an
   AgentMux policy that was already visible to the user.
5. Support session-scoped grants only when both Codex offers the choice and the
   user selects it.
6. Resolve local UI state only after the protocol response write succeeds or a
   `serverRequest/resolved` notification arrives.
7. On interruption, turn completion, or process death, clear every request for
   that turn exactly once.

### 9.2 Permission-mode mapping

Do not carry Claude flag names into App Server. Generate the pinned schema and
define a tested semantic mapping from AgentMux modes to App Server's exact
`approvalPolicy` and `sandboxPolicy` values.

The mapping must preserve these product meanings:

- **Plan/read-only:** no unapproved workspace mutation.
- **Default:** Codex's normal safe policy, including visible requests.
- **Auto:** automatically allow policy-approved operations and prompt for the
  remainder.
- **Bypass:** unrestricted behavior only after the existing explicit AgentMux
  selection; never silently selected because a wire value is unknown.

An unknown AgentMux mode or unsupported App Server policy fails closed.

---

## 10. Authentication and isolated accounts

App Server's account APIs replace terminal-output scraping for Codex login.

### 10.1 Existing account

1. Resolve the agent's canonical `codex` Armory binding.
2. Resolve and validate its `OAuthConfigDir`.
3. Spawn App Server with `CODEX_HOME` set to that exact directory.
4. Complete initialization.
5. Call `account/read` and verify that the server reports a usable account.
6. Block thread creation if the selected home is unauthenticated.

The controller must never fall back to the operator's ambient `~/.codex`.

### 10.2 Connect and re-login

The Codex account UI uses a dedicated App Server spawned under the account home
being created or refreshed. Prefer `chatgptDeviceCode` when supported by the
pinned schema because the frontend owns that ceremony and no callback listener is
required. Browser `chatgpt` login is an allowed fallback.

The flow must:

- show the returned verification URL/code or auth URL;
- wait for `account/login/completed`;
- verify with `account/read`;
- persist/reuse the Armory account only after verification;
- verify the exact canonical Stash link before showing success;
- cancel an outstanding login when the UI is closed;
- never place API keys, access tokens, or refresh tokens in pane events or logs.

API-key login is a separate account kind and must not be conflated with a ChatGPT
subscription account.

### 10.3 Rate limits

`account/rateLimits/read` and update notifications may feed a Codex-specific
status surface after core lifecycle support ships. Rate-limit telemetry is not a
launch dependency and failure to read it must not invalidate a working account.

---

## 11. Native Codex configuration

The provider-native requirements in the August spec remain valid:

- preserve user-owned `AGENTS.md` and base Codex config;
- materialize AgentMux-managed skills in Codex's native skill locations;
- materialize MCP configuration through a Codex-supported config layer;
- deliver persistent agent guidance without generating Claude-only files;
- keep Startup Bundle content in first-turn action context;
- do not present Claude native memory as Codex memory.

App Server adds native discovery and control surfaces (`config/read`, skill
listing/change events, and per-turn settings), but those do not authorize AgentMux
to rewrite user configuration. The provider materializer remains owner of
AgentMux-managed files. App Server reads or receives the result.

For a skill invoked explicitly by AgentMux, include a typed skill input with its
name and path when supported by the pinned schema. This avoids relying only on the
model to resolve a `$skill-name` marker.

---

## 12. Persistence and recovery

AgentMux persists:

- Codex thread ID and last observed turn ID;
- controller generation/epoch;
- normalized document events and terminal status;
- unresolved user requests only while they are genuinely live;
- raw protocol frames subject to secret redaction and retention policy.

AgentMux does not persist access tokens or copy Codex's rollout files into its
database.

After an AgentMux or child-process restart:

1. Resolve the same account and `CODEX_HOME`.
2. Start and initialize a new App Server.
3. Resume the stored thread.
4. Read/reconcile thread history if needed.
5. Mark any pre-crash approval/input request stale; never replay a decision into a
   new controller generation.
6. Return to `Idle` or surface a typed recovery failure.

Automatic restart is bounded. Repeated protocol crashes trip the existing
watchdog/backoff policy rather than entering a spawn loop.

---

## 13. Security requirements

- Use stdio; open no listening socket.
- Spawn with the selected isolated `CODEX_HOME` and explicit workspace cwd.
- Keep analytics disabled by default, matching App Server's documented default.
- Redact auth payloads, tokens, authorization URLs where query parameters may be
  sensitive, and generated secrets from logs.
- Treat stdout as protocol-only. Non-JSON stdout is a framing violation, retained
  in a bounded diagnostic tail.
- Bound line size, pending request count, event size, and stderr retention.
- Never auto-approve an unknown request method or unrecognized permission.
- Validate every file root and cwd crossing the protocol boundary.
- Do not expose `thread/shellCommand` until its out-of-sandbox behavior has a
  dedicated user-consent design.
- Do not enable experimental API capability globally to obtain one desired
  method.

---

## 14. Version and schema contract

For each supported Codex pin:

1. Run the pinned binary's `app-server generate-json-schema` and `generate-ts`.
2. Store the generated artifacts under a versioned directory, proposed as:

   ```text
   schema/providers/codex/app-server/<cli-version>/
   ```

3. Capture sanitized protocol transcripts for handshake, normal turn, command,
   file change, failure, approval, user input, interrupt, steer, resume, and fork.
4. Add a manifest containing CLI version, platform, invocation, scenario, and
   expected terminal state.
5. Generate Rust wire types or validate hand-written types against the committed
   schema; do not use unchecked `serde_json::Value` past the framing boundary for
   known messages.
6. Block a Codex pin bump when schema diff review or fixture replay fails.

The existing `0.116.0` `exec --json` fixtures remain fallback contracts. They do
not prove App Server compatibility for `0.154.0`.

---

## 15. Delivery slices

### Slice 0 — evidence and drift correction

- Capture `0.154.0` generated schemas and sanitized protocol fixtures.
- Record exact stable versus experimental methods used by this spec.
- Update the August Codex specs so their pin/status and App Server deferral do not
  contradict live code or this decision.
- Add a pin/schema consistency gate.

### Slice 1 — controller and handshake

- Add `ControllerType::AppServer` and `AppServerKind::Codex`.
- Implement framed stdio, request correlation, initialization, shutdown, stderr
  tail, timeouts, and crash classification.
- Test against a deterministic fake App Server process.
- Keep the feature disabled by default.

### Slice 2 — threads, turns, and rendering

- Implement thread start/resume and turn start.
- Add notification/item translation with preview reconciliation.
- Persist thread identity and normalized terminal state.
- Replay captured normal, command, file-change, MCP, and failure fixtures.

### Slice 3 — interactive control

- Implement interrupt and steer.
- Implement command, file, permission, user-input, and MCP elicitation requests.
- Add stale-request and cross-pane isolation tests.
- Remove bypass as a technical requirement for App Server operation.

### Slice 4 — Codex account UX

- Generalize the Claude-only account surface where shared lifecycle behavior is
  real, while keeping protocol/auth ceremonies provider-specific.
- Implement App Server login, cancellation, verification, re-login, and logout.
- Verify canonical Armory/Stash links and fail closed on ambient-home leakage.

### Slice 5 — native configuration

- Finish Codex profile/instruction materialization.
- Materialize `.agents/skills` and Codex MCP configuration.
- Connect native skill discovery to AgentMux's Armory/Stash selections.
- Preserve every user-owned file and clean up only manifest-owned artifacts.

### Slice 6 — product lifecycle

- Implement fork, compact, history recovery, rate-limit display, and richer
  status.
- Run host and supported-container end-to-end suites.
- Enable App Server for new Codex panes behind a rollback flag.

### Slice 7 — default and fallback retirement

- Make App Server the default after one release of successful opt-in telemetry-free
  diagnostics and user testing.
- Retain explicit `exec --json` fallback for one additional release.
- Remove fallback only after migration, crash recovery, and authentication gates
  are green on Windows, macOS, and Linux.

---

## 16. Test matrix

### 16.1 Protocol unit tests

- fragmented and coalesced stdout reads;
- CRLF and LF framing;
- malformed JSON and oversized frames;
- client response success/error correlation;
- server request correlation;
- duplicate, missing, null, string, and numeric IDs;
- notification before/after expected response;
- unknown notification versus unknown request behavior;
- EOF and stderr-only failure before and after initialization.

### 16.2 Lifecycle tests

- start -> initialize -> thread -> turn -> idle;
- resume after child restart;
- resume-not-found recovery without silent new thread;
- interrupt while streaming, using a tool, and waiting for approval;
- steer while running and queue while idle;
- fork creates a distinct thread;
- compact renders and returns to idle;
- repeated crash trips bounded recovery.

### 16.3 Rendering tests

- deltas plus completed item render once;
- command stdout/stderr and exit code;
- file diff before and after approval;
- reasoning and plan updates;
- MCP call and result;
- turn failure and process failure;
- usage and context state;
- unknown item retention without pane failure.

### 16.4 Approval/input tests

- accept, accept-for-session, decline, and cancel when offered;
- network-specific request rendering;
- user question with multiple fields and timeout;
- response write failure leaves visible error state;
- `serverRequest/resolved` clears the exact request;
- stale controller generation cannot answer a new request;
- two panes cannot answer each other's requests.

### 16.5 Identity and security tests

- selected `CODEX_HOME` is the child environment;
- ambient home is never used;
- account A and account B remain isolated;
- login cancellation and re-login reuse the intended account directory;
- secrets and auth URLs are redacted;
- unknown permission requests fail closed;
- no network listener is opened;
- unsupported policy mapping never becomes bypass.

### 16.6 Platform and integration tests

- pinned CLI schema generation on Windows and Linux CI;
- authenticated manual smoke on Windows, macOS, and Linux;
- two-turn resume;
- command and file change with approval;
- interrupt and steer;
- process kill and recovery;
- App Server disabled/unavailable -> explicit fallback behavior;
- Docker `CODEX_HOME` projection once the companion Docker slice ships.

---

## 17. Acceptance criteria

Codex App Server support is complete when:

1. A Codex pane runs through one persistent App Server process over stdio.
2. Handshake, thread, turn, item, and terminal states are typed and deterministic.
3. Text/tool previews reconcile with authoritative completed items without
   duplication.
4. Approvals and user-input requests are visible, scoped, answerable, and fail
   closed.
5. Stop and steering use native interrupt/steer operations.
6. Thread resume survives child and AgentMux restarts; Quick Fork creates a real
   upstream fork.
7. The exact linked Armory account supplies `CODEX_HOME` and App Server account
   status.
8. Codex login no longer depends on scraping terminal output.
9. Codex-native instructions, skills, and MCP configuration reach the session
   without modifying user-owned files.
10. Generated schema and fixture gates are tied to the exact pinned CLI.
11. No secret appears in logs, pane events, argv, or persisted normalized events.
12. Windows, macOS, and Linux smoke tests pass, or unsupported platforms are
    blocked honestly.
13. The `exec --json` fallback can be selected during the rollback window and is
    never entered silently after an in-flight App Server turn.

---

## 18. Open questions requiring implementation evidence

1. Which methods used here are non-experimental in the exact `0.154.0` generated
   schema, despite the CLI labeling the App Server command experimental?
2. Does `0.154.0` support `chatgptDeviceCode` on every target platform, and what
   exact cancellation races occur when its UI closes?
3. Which item deltas are guaranteed to receive an authoritative completed item
   after interrupt or provider failure?
4. Can one App Server safely serve multiple threads under one account without
   cross-pane approval ambiguity, and is that benefit worth later pooling?
5. What is the exact mapping from AgentMux permission modes to the pinned
   `approvalPolicy` and `sandboxPolicy` enums?
6. Which config values should be supplied at `thread/start`, at `turn/start`, or
   through an AgentMux-owned profile to preserve prompt caching and user config?
7. Does App Server expose enough history to reconstruct a pane after AgentMux
   loses normalized events, including tool output and interrupted turns?
8. What process/resource envelope does one idle App Server consume compared with
   today's subprocess-per-turn controller?
9. Does Docker use the same stdio controller unchanged once `CODEX_HOME` is
   projected, or does child ownership need a container transport adapter?
10. Should Claude Managed Agents become a separate future provider/runtime for
    remote autonomous work? It must not reuse the local `claude` provider ID.

---

## 19. Normative and research sources

### OpenAI

- [Codex App Server](https://developers.openai.com/codex/app-server/) — intended
  rich-client role, transports, handshake, threads, turns, streaming, approvals,
  user input, skills, account APIs, and schema generation. Retrieved 2026-09-12.

### Anthropic

- [Claude Agent SDK overview](https://platform.claude.com/docs/en/agent-sdk/overview)
  — local library role, supported languages, capabilities, authentication
  restriction, and comparison with Managed Agents. Retrieved 2026-09-12.
- [Claude Managed Agents overview](https://platform.claude.com/docs/en/managed-agents/overview)
  — hosted agent/session/environment model, stateful sessions, SSE events, tools,
  and beta status. Retrieved 2026-09-12.
- [Managed Agents event stream](https://platform.claude.com/docs/en/managed-agents/events-and-streaming)
  — bidirectional events, deltas, steering, interrupt, persistence, and session
  viewing. Retrieved 2026-09-12.
- [Managed Agents self-hosted sandboxes](https://platform.claude.com/docs/en/managed-agents/self-hosted-sandboxes)
  — customer-hosted execution with Anthropic control-plane orchestration and data
  flow. Retrieved 2026-09-12.
- [Claude Platform release notes](https://platform.claude.com/docs/en/release-notes/overview)
  — Managed Agents public-beta launch and continuing feature releases, including
  permissions and session connection. Retrieved 2026-09-12.

No roadmap claim in this spec is inferred from community posts or third-party
reporting. The absence of an announced local Claude App Server is explicitly
time-bounded to the official sources checked on 2026-09-12.
