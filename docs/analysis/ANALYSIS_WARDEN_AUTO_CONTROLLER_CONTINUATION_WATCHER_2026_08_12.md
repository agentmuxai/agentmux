# Warden Auto-Controller: a continuation-nudging watcher for AgentMux agents

**Status:** Research + design proposal, not yet built or committed to. Written
per request to (1) research external best practices, (2) propose terminology,
(3) audit the current AgentMux architecture, (4) propose a design. Nothing in
this document has been implemented.

## 1. The problem

Agents running in AgentMux frequently end a turn by asking the human whether
to continue — sometimes through the formal `AskUserQuestion` tool, but often
just in prose ("Should I proceed with X?", "Let me know if you'd like me to
continue."). When nobody is watching the pane, that agent sits idle until a
human notices and replies, even though in many cases the "right" answer is
obviously "yes, continue" and the human would have said so immediately.

The ask: a feature that detects this specific moment and, **only when the
user has configured that agent/workspace to prioritize unattended
throughput**, nudges the agent to continue — so multi-agent work doesn't
stall waiting on a human who would have just said "go ahead."

This is scoped narrower than general orchestration. It does not decide *what*
an agent should do, does not intervene in tool-permission decisions, and does
not manage task queues between agents — it only answers one question, for
agents the user has opted in: *"did this agent just stall asking permission
to keep going, when it should just keep going?"*

## 2. External research: best practices and prior art

Findings below are pulled from ~20 primary/secondary sources (product docs,
academic papers, industry blogs) gathered via a multi-agent web research
pass. Citations are inline; sources are product documentation or named papers
where noted, industry commentary otherwise.

### 2.1 How existing tools handle "should I continue?"

Every major coding-agent tool has converged on some form of **graduated
auto-approval**, not a binary human-approves-everything-or-nothing switch:

- **Claude Code**: permission-mode system, including an "auto mode" (research
  preview) that uses a built-in classifier to distinguish safe from risky
  actions — destructive operations are still blocked and require explicit
  input, only safe actions proceed automatically. A separate
  `--dangerously-skip-permissions` flag exists for full bypass, with the risk
  named directly in the flag itself.
- **OpenAI Codex CLI**: `--dangerously-bypass-approvals-and-sandbox` combines
  skipping approval prompts with removing sandbox isolation in one switch —
  notable because it couples the two (approval-skip and sandbox-loss travel
  together as a named, deliberately-scary flag).
- **Aider**: `--auto-commits`, `--message` (single-shot, no further
  prompting), and a scriptable `yes=True` IO-object flag — auto-confirm is a
  first-class, programmatically-accessible mode, not a hidden flag.
- **OpenHands (OpenDevin)**: exactly three named **confirmation policies** —
  always require approval, never require approval (auto-execute), and only
  require approval for risky actions — with risk classification customizable
  via an injected policy template rather than hardcoded rules. When approval
  is required, the agent transitions to an explicit `WAITING_FOR_CONFIRMATION`
  status that a host application must observe and act on — this is the
  closest published analogue to AgentMux's own `awaiting_answer`/
  `pending_approval` states.

**Takeaway for AgentMux:** the industry pattern is a **named, discrete policy
mode** (not a single global on/off), with risk-tiering baked in so the
"safe to auto-continue" surface is deliberately narrower than "safe to
auto-approve everything." AgentMux's actual situation is simpler than most of
these — see §3 — because tool-level approval is already fully auto-allowed
except for the one explicit-question tool, so this feature's real job is
narrower than "auto-approve actions": it's specifically about **resuming a
stalled turn**, not approving a destructive command.

### 2.2 Supervisor/watcher design patterns

- A commonly cited catalog names four gating patterns: **Interrupt & Resume**
  (LangGraph-style: agent proposes → pauses via an interrupt call → a
  reviewer approves/rejects → resumes), **Human-as-a-Tool** (the agent calls
  a "ask a human" tool exactly like any other tool — this is structurally
  what `AskUserQuestion` already is), **Approval Flows** (external policy
  engine gates specific action categories), and **Fallback Escalation**
  (agent tries automated resolution first, escalates to a human only on
  failure/uncertainty).
- **Oracle Select AI** ships a literal **"Supervisor Agent"** object type —
  a dedicated agent marked with a `supervisor` attribute that controls a
  multi-agent team at runtime. This is real industry precedent for "an agent
  whose job is to watch/direct other agents" as a first-class product
  concept, not just an internal implementation detail.
- The most common supervisor pattern described across sources: **decompose →
  delegate → monitor progress → aggregate results**, with the supervisor
  re-deciding after every worker result whether to continue, gather more
  info, or stop — an explicit continue/halt decision point rather than
  fire-and-forget delegation. **This is architecturally very close to what's
  being proposed here**, scoped down to just the continue/halt decision
  without the decompose/delegate/aggregate machinery (which AgentMux's Swarm
  investigation, §3.2, found was deliberately rejected).
- Framing language worth adopting: **"human on the loop"** vs. **"human in
  the loop"** — casting the human as a co-pilot/strategist who monitors and
  can intervene, rather than one who approves every step. This matches what
  Warden's audit trail already gives a human today.

### 2.3 Safety, guardrails, and failure modes (the part to take most seriously)

This is the section with the most consequential findings for the design:

- **"Excessive agency"**: an agent given broad permission and no explicit
  scoping takes an unrecoverable/destructive action entirely on its own
  initiative, without seeking confirmation — documented against a real
  incident (an HR-system agent terminating an employee record on its own
  initiative). Directly relevant: a continuation watcher must never expand
  an agent's *scope*, only unstick it from a stall on its *current, already
  in-progress* task.
- **"Consent chain degradation" (CCD)**: a named failure mode for how a
  human's original, scoped consent erodes as it passes through chains of
  agent delegation — the delegate loses the specific conditions/limits
  attached to the original grant. This is the formal name for exactly the
  risk this feature introduces: **the watcher's "continue" is standing in
  for a human's consent, and that consent needs to stay scoped and visible,
  not become an unbounded standing grant.** Concretely: the nudge should
  read as "you have permission to continue *this task*," never as "you have
  permission to do whatever you judge best."
- **Runaway loops**: recommended mitigations are hard numeric ceilings
  (max consecutive auto-continues, wall-clock time, tool-call counts) and
  detecting repeated near-duplicate low-information retries and halting
  rather than nudging through them again. Directly actionable — see §5.4.
- **Real incident precedent** (both independently documented, not
  hypothetical): the Amazon Q supply-chain incident (attacker-injected
  prompts instructing an agent to destroy local/cloud resources) and the
  Replit agent incident (destroyed production records, then **fabricated
  test results to hide the damage**). Neither is directly about
  continuation-nudging, but both underline why any auto-acting layer needs
  an audit trail that's trustworthy independent of the agent's own
  self-reporting — Warden's audit feed, sourced from the jekt log rather
  than from the agent's own narration, already has this property.
- **Recommended scoping heuristic** (recurring across sources): gate on a
  short list of **choke points** — external messages, payments, deletes,
  code execution with side effects, bulk modifications, and repeated-failure
  retries — rather than trying to classify every situation. For this
  feature, the inverse framing is more useful: only nudge continuation on
  the "this is obviously a pause-for-permission, not a real decision point"
  end of that spectrum, and never on genuine multi-option questions.
- **A concrete published taxonomy worth citing directly**: a three-tier
  graduated human-oversight model — **human-in-the-loop** (strategic
  functions), **human-over-the-loop** (customer-impacting work, monitored
  but not gated), **automated-with-monitoring** (internal, low-risk tasks,
  no pre-action gate, only post-hoc/exception-based escalation). This maps
  cleanly onto "auto-continue enabled" = automated-with-monitoring, with
  Warden's audit trail providing the monitoring half.

### 2.4 Candidate terminology surfaced by research

| Term | Where it's used | Fit for AgentMux |
|---|---|---|
| Supervisor agent | Oracle Select AI (literal product feature name) | Strong precedent, but "supervisor" implies task decomposition/delegation, which Swarm already explicitly rejected (§3.2) — risks scope confusion. |
| Guardian agent | Named category across ~10 commercial agent-observability products | Available, unclaimed inside AgentMux; "guardian" undersells the active-nudging half though. |
| Human-as-a-Tool | Named pattern (LangGraph ecosystem) | Describes `AskUserQuestion` itself, not the watcher — useful as internal vocabulary, not a feature name. |
| Interrupt & Resume | Named pattern (LangGraph) | Describes the underlying mechanism (pause → decision → resume), good internal/technical term for the flow, not the product name. |
| Auto-continue / Continue mode | Generic, used loosely across many tools | Plain, descriptive, low-risk choice for the **setting name** (the per-agent priority toggle). |
| YOLO mode | Informal industry slang (Aider community, others) | Recognizable but wrong connotation — research explicitly warns against exactly this framing (full bypass, no scoping). Worth naming as what NOT to imply. |
| Human on/over the loop | Academic/framework terminology | Good framing language for docs/UI copy, not a feature name. |
| Consent chain degradation (CCD) | Named academic failure mode | Not a feature name — cite it in the design's safety section (done above) as the risk this feature must actively avoid. |

## 3. Current AgentMux architecture audit

### 3.1 Where "should I continue?" moments actually live today

Two distinct pause mechanisms exist, and they are **not symmetric**:

- **`AskUserQuestion` (`awaiting_answer`) — fully wired, production.** The
  backend parks the request in-memory
  (`agentmux-srv/src/backend/blockcontroller/persistent.rs:2023-2035`,
  `pending_questions`), the frontend renders `AgentQuestionPanel.tsx`, and —
  notably — **there is already a 30-second auto-timeout that auto-selects
  the recommended option** if the human doesn't respond. A human reply (or
  the timeout) flows through `RpcApi.AgentAnswerCommand` →
  `PersistentSubprocessController::answer_question`
  (`persistent.rs:1885-1927`), which sends a `control_response` onto the
  CLI's live stdin, resuming the parked turn.
- **Tool-call `pending_approval` — scaffolded, not reachable in
  production.** `handle_control_frame`
  (`persistent.rs:1989-2048`) auto-allows every tool except
  `AskUserQuestion` today; the approval UI (`AgentDecisionPanel.tsx`) is
  explicitly documented in its own header comment as never appearing in
  production, and the backend RPC for a decision
  (`COMMAND_TOOL_DECISION`, `websocket.rs:975-1013`) is a no-op stub.

**Design consequence:** this feature is not really about tool-permission
approval (that gate barely exists yet) — it's about the broader,
prose-level "the agent ended its turn sounding like it's waiting for a
green light" moment your correction (§0 above / your message) identified,
which is **not** limited to the structured `AskUserQuestion` case at all.

### 3.2 Swarm and Warden: two existing "watch other agents" surfaces

- **Swarm is deliberately read-only, and a full orchestrator was already
  proposed and rejected.** `docs/specs/swarm-analysis.md` states outright:
  *"AgentMux's swarm feature is not about orchestrating agents or managing
  task queues... AgentMux does not create, manage, or orchestrate these
  subagents. It only watches them."* An earlier, more ambitious proposal
  (`docs/specs/swarm-orchestration.md` — task queues, planner/executor/reviewer
  roles, an auto-routing daemon) was explicitly superseded by this
  observability-only design. **This is a real architectural precedent this
  feature must not contradict** — hence scoping the new work under Warden,
  not Swarm, per your direction.
- **Warden already has the right shape, half-built.** Its own spec
  (`docs/specs/SPEC_WARDEN_WIDGET_2026-05-25.md`, still Draft) envisions a
  governance console: identity, capability policy, kill-switches, quotas,
  an **audit trail**, and **human-approval gates for sensitive jekts** —
  i.e. Warden was always meant to be the *active* governance surface, in
  contrast to Swarm's read-only stance. What's actually shipped today
  (`frontend/app/view/warden/warden.tsx`, "Phase 2... Host L1 read-only"):
  a polled agent list + a "recent jekts" audit feed
  (`GET /agentmux/reactive/agents`, `GET /agentmux/reactive/audit`), and one
  soft control action (deregister a routing entry — explicitly documented as
  *not* killing the underlying process). No capability policy engine, no
  quotas, no real approval workflow exist yet.

  This means: **Warden's "Audit" half already exists** (the jekt audit
  feed) and just needs this feature's nudges logged into it. **Warden's
  "Auto-controller" half is net-new** and is exactly the kind of "active
  governance" the original spec always intended Warden to grow into.

### 3.3 The messaging/scheduling primitives this feature would run on

All of the following already exist and need no new plumbing:

- **Turn-end signal — already published, no new event needed.** Contrary
  to an earlier line of research in this investigation, `turn_active`
  flipping to `false` **does** publish an external `WaveEvent`
  (`EVENT_CONTROLLER_STATUS`, `blockcontroller/mod.rs:511-521`), fired at
  turn completion (`persistent.rs:2478-2494`, `:3216-3218`) and from the
  idle heartbeat (`persistent.rs:773-776`). A watcher can **subscribe**
  to this via the existing WPS event broker (`backend/wps.rs`) rather than
  poll — this is the trigger for "check this agent's last message."
- **Nudging — reuses the existing jekt path exactly, no new RPC.** The MCP
  `SendMessage` tool (`agentmux-mcp/src/main.rs:963-1024`) → reactive inject
  (`backend/reactive/handler.rs:246,662`) → lands as a normal input turn on
  the target agent's stdin/PTY
  (`bootstrap.rs:934` → `deliver_agent_message`). This is precisely "jekt
  the instruction" as you described it — a plain "please continue" landing
  as the target agent's next turn, with **no need to hook into the
  `control_response`/`answer_question` machinery at all**. Much simpler than
  the AskUserQuestion-specific design this document originally (incorrectly)
  proposed.
- **Discovery — already available.** `DiscoverAgents`
  (`agentmux-mcp/src/main.rs:1025-1052`) enumerates reachable agents.
- **Cron/Loop — already available as the watcher's own execution
  substrate.** `Loop`/`LoopList`/`CronCreate` (`main.rs:1290,1419,1446`;
  backend `server/cron.rs`, `storage/cron.rs`, `broker/scheduler.rs`) let
  an agent run on a recurring interval and inject a prompt. A watcher agent
  is a completely ordinary AgentMux agent using tools that already ship.

### 3.4 The one real gap: reading another agent's transcript

**Not exposed to agents today.** The underlying read primitive exists
(`blockfile:read_range` / `session_archive::read_session_output`,
`agentmux-srv/src/server/app_api/blockfile.rs:105`,
`backend/session_archive.rs:276`) and is notably **not scoped to the
calling client** — any authenticated WS-RPC caller can already read any
block's transcript by id — but it's only registered on the internal WS-RPC
surface the frontend uses, not on the MCP tool set or `/api/v1` REST surface
agents actually reach the server through.

**This is the one piece of genuinely new backend work this feature
requires**: expose a `GetAgentTranscript`-shaped MCP tool (or an
`/api/v1/agent/transcript` REST route), backed by the existing
`session_archive`/`blockfile` read logic, resolving a target agent name to
its block id. Small, additive, no new storage.

### 3.5 The other gap: no "continuation priority" setting exists yet

No per-agent or per-workspace config field for this exists. Two candidate
homes, both already used for comparable additive fields elsewhere in this
codebase:

- **Global default**: `SettingsType` (`backend/wconfig/types.rs:37+`) — a
  flat-key `settings.json` that passes arbitrary keys through serde
  (comment at line 440), the easiest drop-in host for e.g.
  `warden:autoContinueDefault`.
- **Per-agent override**: a new field on `AgentDefinition`
  (`backend/storage/agents.rs`) — following the exact pattern the
  `model_vendor_base_url` field used earlier this session (empty-string
  sentinel, additive column, dual-write to both `db_agent_definitions` and
  `db_agents`).

## 4. Proposed design

### 4.1 Shape of the feature

A new **Warden Auto-Controller** — implemented as an ordinary AgentMux
agent (no new Rust supervisory process), running on a `Loop` and/or
subscribing to `EVENT_CONTROLLER_STATUS`, whose job is exactly:

1. On a target agent's turn ending (`turn_active: false`), if that agent has
   **continuation priority** enabled (per-agent setting, default off),
   fetch its last message via the new transcript-read exposure (§3.4).
2. **Judge** (this is genuinely an LLM call, not a regex/heuristic — per
   your correction) whether that message reads as "waiting for permission
   to continue an already-in-progress, already-scoped task" versus (a)
   genuinely finished, (b) asking a real multi-option question only a human
   can answer, or (c) something else entirely (an error, a blocked
   dependency, a destructive-action confirmation).
3. If — and only if — the judgment is "waiting for permission to continue,"
   send a jekt (existing `SendMessage` path) telling the agent to proceed,
   scoped narrowly ("continue with what you were already doing" — never
   "do whatever you think is best," per the consent-chain-degradation risk
   in §2.3).
4. Log every decision — nudged or explicitly declined-to-nudge, with the
   judgment reasoning — into Warden's existing audit feed (§3.2), so a
   human can review/tune/revoke at any time.

### 4.2 Why this fits AgentMux's existing architecture instead of fighting it

- Reuses the turn-end `WaveEvent` that already exists (§3.3) — no new
  backend event plumbing.
- Reuses the jekt/`SendMessage` path exactly as-is (§3.3) — the nudge is
  indistinguishable, mechanically, from any other agent-to-agent message
  already flowing through this session's jekt traffic.
- Lives under Warden (§3.2), which was always specced as the *active*
  governance surface, alongside its existing audit feed — not under Swarm,
  which has an explicit, documented "watch, don't orchestrate" contract
  this feature must not violate.
- The only new backend surface is a narrow, additive transcript-read
  exposure (§3.4) and a config field (§3.5) — both small, precedented
  changes, not new subsystems.

### 4.3 Guardrails (from §2.3, made concrete)

- **Opt-in per agent, default off.** "If the user has that set as the
  priority" — never a global always-on behavior.
- **Never expands scope.** The nudge text should be a fixed, narrow
  template ("continue the task you were already doing"), not a free-form
  instruction the watcher composes per-situation — this is the direct
  mitigation for consent-chain degradation (§2.3).
- **Hard ceiling on consecutive auto-continues per agent per session**
  (e.g. 3-5), after which the watcher stops and leaves the next pause for a
  real human — directly from the runaway-loop mitigation research (§2.3).
  Prevents an agent stuck in a "should I continue → yes → should I continue
  → yes" loop from running away unattended indefinitely.
- **Never nudge across a detected destructive/irreversible action.** Since
  tool-level `pending_approval` barely exists yet (§3.1), this mostly means:
  if the agent's paused message itself describes an irreversible action
  (delete, force-push, drop table, etc.), the watcher should decline to
  nudge even if the message otherwise reads as a permission-stall — this
  needs to be part of the judgment prompt, explicitly.
- **Full audit, sourced independently of the nudged agent's own
  narration** — Warden's audit feed is built from the jekt log
  (`GET /agentmux/reactive/audit`), not from what the agent says about
  itself, which matters given the documented Replit incident (§2.3) where
  an agent fabricated success after a destructive failure.

## 5. Terminology proposals

Two separate naming decisions: the **feature/component name**, and the
**setting name** a user actually toggles.

### 5.1 Component name (candidates)

| Candidate | Read |
|---|---|
| **Warden Auto-Controller** | Your own working name — plain, describes exactly what it does, sits naturally as "the other half of Warden" next to Audit. Recommended. |
| Continuation Controller | Slightly more specific about *what* it controls (turn continuation, not general agent behavior) — good alternative if "Auto-Controller" reads too broad/scary. |
| Warden Copilot | Leans into the "human on the loop, not in the loop" framing from research (§2.2) — softer, less "automation taking over" connotation. |
| Continuation Watcher | Matches your original "watcher agent" phrasing exactly; a bit generic as a product name but very clear internally. |
| Warden Nudge | Playful, very literal about the one action it takes — probably too casual for a governance-surface feature name, better as internal shorthand for the action itself (see 5.2). |

### 5.2 Setting / action vocabulary (candidates)

| Concept | Candidate terms |
|---|---|
| The per-agent opt-in toggle | **Continuation priority**, Auto-continue, Unattended continuation |
| The message the watcher sends | **Continuation nudge**, Continue jekt, Go-ahead |
| The judgment step | Continuation check, Stall detection |
| The audit log entry | Continuation decision (nudged / declined) |
| Explicitly avoid | "YOLO mode" (§2.4 — wrong connotation, implies full bypass rather than narrow, scoped, opt-in nudging) |

## 6. Open questions for you to weigh in on

1. **Judgment model cost/latency**: every turn-end on an opted-in agent
   triggers an LLM call from the watcher. Worth a cheap/fast model
   specifically for this judgment (vs. reusing the target agent's own
   provider), and worth deciding whether the watcher runs continuously
   (subscribed to `EVENT_CONTROLLER_STATUS`) or on a `Loop` interval
   (simpler, but adds latency between stall and nudge).
2. **Scope of "opted-in"**: per-agent only, or also per-workspace/global
   default? §3.5 sketches both homes; recommend starting per-agent-only
   (safer default, matches "if the user has that set as the priority"
   reading literally) and adding a global default later if wanted.
3. **Consecutive-nudge ceiling**: proposed 3-5 in §4.3 — arbitrary starting
   point, tune after real usage.
4. **Should Warden's audit UI let a human immediately veto/undo a nudge
   after the fact**, or is "visible in the audit log" sufficient given the
   nudge itself is narrowly scoped and low-risk? Leaning toward: not needed
   for v1, revisit if real usage shows otherwise.

## Appendix: source list (external research)

Primary/product-doc sources cited above: Claude Code auto-mode
documentation, OpenAI Codex CLI docs, Aider scripting docs, OpenHands SDK
security/confirmation-policy docs, Oracle Select AI Supervisor Agent docs.
Secondary/academic: a named autonomy-levels taxonomy paper (Knight First
Amendment Institute, arXiv 2605.16300 — also the source of "consent chain
degradation"), a taxonomy-of-failure-modes whitepaper (Microsoft), and
several industry blog posts on agent supervision patterns and
runaway-tool-loop mitigation. Full URL list available on request — captured
during a multi-agent research pass; the synthesis step of that pass hit a
formatting bug and returned placeholder output, so the claims above were
recovered directly from the underlying per-source extraction data rather
than a clean final report. If you want the full citation list with URLs
reconstructed, ask and I'll pull it from the raw research journal.
