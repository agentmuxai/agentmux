# Spec: Per-Tool-Call Permission Decision Prompt

**Date:** 2026-04-24
**Status:** Draft, ready for implementation planning
**Owner:** AgentA
**Tracks:** agentmuxai/agentmux#551 ("Decision prompt: per-tool-call
            permission UI for agent panes") + user follow-up:
            "remember the options the user last set"
**Touches:** `agentmux-srv/src/server/websocket.rs`,
             `frontend/app/view/agent/` (new decision panel +
             event types + state), `frontend/app/store/` (new
             memory store), settings-template.

---

## 1. Problem

AgentMux gates tool calls **before** an agent turn starts (via
`PermissionMode` → CLI flag in `frontend/app/view/agent/
buildRuntimeArgs.ts:18-24`). When the user picks any mode other
than `bypass`, the agent CLI asks the subprocess's stdin for
`y/n` on each tool call — but AgentMux isn't listening, doesn't
surface the question, and the pane stalls indefinitely in
`running` (⏳).

Operationally, this means the only viable mode today is `bypass`
(`--dangerously-skip-permissions`), which defeats the point of
having a permission system at all.

Plus: even if we build the decision UI, a prompt that appears
for every tool call in every session — with no memory of what
the user already approved last time — is a worse UX than bypass.
The user's **second** decision for the same tool on the same
target should take at most one keystroke, and their **third**
should often be automatic.

## 2. Goals

- **G1.** When the agent issues a tool call and current mode
  isn't `bypass`, surface a visible decision prompt within
  100 ms. Pane shows as "awaiting approval" (distinct from
  `running`).
- **G2.** Keyboard-driven approval path that doesn't force the
  user to leave the composer. `Enter` = accept, `Shift-Enter` =
  deny with feedback, letter hotkeys for scope selection.
- **G3.** Scoped allow-rules persist and are honoured on the
  next session start. Project-scoped rules survive reopening the
  repo; global rules survive reinstalls.
- **G4.** **Remember the user's last-set options** (this spec's
  extension to #551): when the same tool-target combination
  reappears and no rule auto-approves it, the prompt defaults to
  whichever action the user picked last time, with the same
  scope pre-selected. Multi-level memory: per-(tool, target), per
  agent-pane, per user.
- **G5.** High-risk prompts (destructive shell, writes outside
  repo, network side effects) cannot be one-keystroke approved
  — require an additional confirm step.
- **G6.** Denials carry user-typed feedback verbatim to the agent
  so the model can actually course-correct on the next turn.

## 3. Non-goals

- **No replacement of pre-turn `PermissionMode`**. That flag
  still exists and still sets the CLI's fallback behaviour;
  per-call prompts only fire when the CLI asks for one.
- **No protocol uplift for CLIs that don't emit per-call
  requests**. Gemini / Kimi stay bypass-only until upstream
  surfaces a request event.
- **No partial approval UI** (e.g. "allow this tool but only
  these args"). Open question in §10.
- **No telemetry export**. The audit log stays on disk; a future
  spec can decide if and how to surface it.

---

## 4. Event model

### 4.1 New stream event

Provider-adapter layer emits a new stream event when the CLI
requests permission:

```ts
export interface PermissionRequestEvent {
    type: "permission_request";
    request_id: string;        // opaque; echoed in decision
    tool: string;              // "bash", "str_replace", "write_file", …
    target: string | null;     // path, command text, url — normalised
    reason?: string;           // provider-supplied rationale
    preview?: PermissionPreview; // diff / bash text / …
    risk?: "low" | "medium" | "high"; // heuristic, defaults "medium"
}
```

`PermissionPreview` is a discriminated union: `{ kind: "diff",
content: string }`, `{ kind: "bash", command: string }`,
`{ kind: "text", content: string }`, `{ kind: "none" }`.

### 4.2 New tool-call state

`ToolCallEvent.status` extends its enum:

```ts
status: "running" | "pending_approval" | "success" | "failed" | "denied"
```

`pending_approval` renders the tool row with an amber
highlight + a "Decide" affordance that focuses the decision
panel on click.

### 4.3 Decision RPC

New websocket method on the sidecar:

```
tool:decision {
    request_id: string;
    outcome: "allow" | "deny";
    scope:   "once" | "session" | "project" | "global";
    rule_matcher?: { tool: string; target_pattern?: string };
    feedback?: string; // only for deny
}
```

The sidecar's `BlockController` forwards the outcome to the
CLI's stdin in whatever form the CLI expects (`y` / `n` for
Claude Code, JSON for Codex, etc.). Provider adapter handles
translation.

---

## 5. UX

### 5.1 Decision panel

Renders as a DOM overlay anchored to the focused agent pane
(position: absolute, bottom-anchored above the composer — the
same real-estate as the SlashHelpPanel). Uses `usePaneOverlay`
so the panel clips through any browser pane HWND.

Layout:

```
┌ ⚠ Decision required ──────── high-risk ──────┐
│ Tool:   bash                                  │
│ Target: `rm -rf node_modules`                 │
│ Why:    "Removing stale dependencies before   │
│          fresh install"                       │
│ ┌ preview ─────────────────────────────────┐  │
│ │ $ rm -rf node_modules                    │  │
│ └──────────────────────────────────────────┘  │
│                                               │
│ Scope:  ( ) once  ( ) session                 │
│         ( ) project  (•) global               │
│                                               │
│ [Allow]   [Deny + feedback]   [Defer]         │
└───────────────────────────────────────────────┘
```

Keyboard:
- `Enter` — run the primary action (Allow for low/medium risk;
  disabled for high risk until a modifier is held)
- `Shift-Enter` — activate Deny + feedback
- `o` / `s` / `p` / `g` — set scope
- `Esc` — Defer (prompt stays; pane stays awaiting)
- `Tab` — cycles focus through the three buttons

### 5.2 High-risk confirm

When `risk === "high"`, Allow is disabled until the user either:
- Presses `Enter` twice (double-tap, 500ms window)
- Or holds `Shift` while clicking Allow

The second step makes auto-approval-by-habit impossible for the
most dangerous operations.

### 5.3 Multi-call queue

If a turn triggers multiple tool calls in quick succession, each
gets its own `permission_request` event. The panel shows the
oldest pending one; a chevron in the header indicates "N more
queued" and the user can walk the queue with `Alt-↑ / Alt-↓`.

Deciding a prompt advances the queue; an empty queue returns
the pane to `running` / `idle`.

---

## 6. Scoped rules (G3)

Rules live in two files:

- `<project>/.agentmux/permissions.json` — project-scoped
- `~/.agentmux/permissions.json` — global

Shape:

```json
{
  "version": 1,
  "rules": [
    {
      "tool": "bash",
      "target_pattern": "^npm test",
      "outcome": "allow",
      "added_at": "2026-04-24T20:00:00Z",
      "added_by_scope": "project",
      "source_request_id": "abc123"
    }
  ]
}
```

Match order: project rules before global. First match wins.
Deny rules are supported and beat allow rules with the same
specificity (hand-tuned tie-breakers documented per case).

Rules are append-only; UI for listing / revoking lives in the
settings pane (future work, not blocking for v1).

---

## 7. Remember-last-options (G4 — extension to #551)

**The issue spec covers *rules*. This section covers *UI
preferences that aren't rules*.** A user who picked
"Allow → once" for `bash: ls ~/Desktop` doesn't want a rule
committed, but on the next matching prompt they don't want to
navigate back to "once" either — they want the panel to
remember what they did last time and default to it.

### 7.1 Three layers of memory

Ordered from most-specific to least:

1. **Per-(tool, target) memory.** Last-chosen outcome + scope
   for a specific tool / target pair. Applies only when no rule
   auto-approves. Indexed by `(tool, target_pattern)`; the
   target is hash-normalised (path segments stripped to basename
   where appropriate, commands trimmed) so trivial variation
   doesn't blow the cache.

2. **Per-agent-pane memory.** Last-chosen *scope* in this pane's
   lifetime. Used when the per-(tool, target) entry is missing
   — pre-select the scope radio the user last picked for *any*
   tool in this pane. Matches the mental model "I've been
   mostly saying 'session' today".

3. **Per-user global memory.** Last-chosen scope ever. Used as
   the cold-start default for a new pane that has no session
   memory yet. Overrides the spec default (`once`) so the first
   prompt in any new pane already reflects the user's habits.

### 7.2 Feedback memory

Deny-with-feedback remembers the **last 5 feedback strings**
the user typed. When the user lands on the feedback input with
no rule match, a dropdown of recent phrasings appears (ArrowDown
to reveal). Useful when the same class of tool call gets the
same rejection reason across turns.

### 7.3 Storage

```
~/.agentmux/decision-memory.json
```

Shape:

```json
{
  "version": 1,
  "last_scope_ever": "session",
  "per_tool_target": [
    {
      "key": "bash:basename(ls)",
      "outcome": "allow",
      "scope": "once",
      "at": "2026-04-24T20:05:00Z"
    }
  ],
  "recent_feedback": [
    "Don't delete node_modules, just npm ci",
    "…"
  ]
}
```

Per-agent-pane memory lives in-memory only (no disk); it resets
when the pane closes. This is intentional — per-pane memory is
a UX convenience, not a durable preference.

### 7.4 Interaction precedence

When a `permission_request` arrives, apply in order:

1. **Rule match** (§6) → auto-respond, no UI.
2. **Per-(tool, target) memory** → show prompt with that outcome
   + scope pre-selected. The user can press `Enter` immediately
   to repeat, or change selection.
3. **Per-agent-pane memory** → show prompt with that *scope*
   pre-selected (outcome defaults to Allow for low-risk, unselected
   for high-risk).
4. **Per-user global memory** → show prompt with the user's
   last-ever scope pre-selected.
5. **Spec default** → scope = `once`, outcome = Allow (low-risk)
   / unselected (high-risk).

The panel's header includes a muted strip when memory was used:
`"Your last choice: Allow · once"` so the user sees the defaults
aren't arbitrary.

### 7.5 Memory vs. rules

Memory and rules are different beasts and don't collide:

| | Rule | Memory |
|---|---|---|
| Triggers UI? | No — auto-decides | Yes — pre-fills the UI |
| Persists where | `.agentmux/permissions.json` | `decision-memory.json` |
| Scope | Matches permanently | One prompt, then updated |
| User intent | "I'm OK with this always" | "Last time I said X, let me say it again with one keystroke" |

A user who wants a rule commits via the scope radio (Allow +
project/global). Memory is the path for users who prefer to
re-confirm each time but don't want to re-navigate the panel.

---

## 8. Settings

New `decisionPrompt` block in `settings-template.jsonc`:

```jsonc
{
  "decisionPrompt": {
    // Disable the prompt entirely (reverts to today's hang-on-request
    // bug). Escape hatch for users who preferred the old behaviour.
    "enabled": true,

    // Auto-allow scope to bypass the memory path. "never" always
    // asks; "low-risk" auto-allows low-risk tools after the memory
    // picks one; "all" mirrors bypass mode at UI level.
    "autoAllow": "never",

    // How many previous feedback strings to remember.
    "recentFeedbackLimit": 5,

    // High-risk confirmation mode: "double-enter" | "shift-click" |
    // "both" (default)
    "highRiskConfirm": "both"
  }
}
```

---

## 9. Provider adapters

Today each provider has an adapter under
`frontend/app/view/agent/translators/`. Add a
`parsePermissionRequest(raw): PermissionRequestEvent | null` hook
per adapter:

| Provider | Support status |
|---|---|
| Claude Code | Stream event `permission_request` exists in the CLI's JSON; map directly |
| Codex | Uses a CRUD-style tool request event; map |
| ACP (OpenClaw, Pi) | JSON-RPC `tools.requestPermission`; map |
| Gemini | No per-call event as of v0.34; stays bypass-only |
| Kimi | Same; bypass-only |
| Copilot | Interactive/Plan/Autopilot switch at the CLI level; map the Interactive mode's per-call prompts |

Unsupported providers gate the prompt feature off in the agent
card's UI so the user can't pick a mode the CLI can't honour.

---

## 10. Open questions

1. **Partial approval.** Should the UI allow editing the tool
   args before approving? "Allow `rm node_modules/` but not
   `rm -rf`" — non-trivial because the CLI's protocol usually
   doesn't support arg mutation. Defer until a use case.

2. **Cross-agent rule inheritance.** If Agent A's rules allow
   `bash: npm test`, should Agent B (different pane, same
   project) inherit it? Default answer: yes, project rules are
   shared. But per-(tool, target) *memory* is per-pane because
   the user might be running two agents with different
   expectations in the same repo.

3. **"Reason" requirement on deny.** Should Deny require typed
   feedback (block empty submissions)? Lean yes — an empty
   denial tells the model nothing actionable. But users pressing
   Esc-as-muscle-memory would find that friction annoying.
   Compromise: require feedback when deny is scoped beyond
   `once`.

4. **Audit log format.** The issue proposes JSONL at
   `.agentmux/decisions.log`. Fine for v1. Rotation / TTL
   unspecified — rotate weekly, retain 90 days.

5. **Memory staleness.** Per-(tool, target) memory entries older
   than 30 days should expire — the user may not remember why
   they said what they said, and a stale pre-fill is worse than
   no pre-fill. Background GC sweep on app start.

---

## 11. Rollout

| Phase | Scope |
|---|---|
| **v1** | Decision panel + Claude Code adapter only. Rules (project + global) + per-(tool, target) memory. `settings-template` gains the block. High-risk double-confirm. Audit log. |
| **v1.1** | Codex + ACP adapters. Recent-feedback dropdown. Memory GC. |
| **v2** | Rule-listing / revoking UI in settings pane. Partial approval research (may defer further). Copilot interactive-mode adapter. |

## 12. Validation / acceptance

- ✅ With a non-bypass mode, issuing a tool call surfaces the
  prompt within 100 ms in the focused pane.
- ✅ A project-scoped allow rule persists across app restarts.
- ✅ Re-prompting on the same (tool, target) defaults to the
  last-chosen outcome and scope.
- ✅ A new pane's first prompt defaults to the user's
  last-ever scope (from global memory).
- ✅ Deny + feedback delivers the typed string to the model's
  next turn (inspect the agent's follow-up message).
- ✅ `Enter` approves in one keystroke for low-risk; needs a
  second action for high-risk.
- ✅ `Alt-↑ / Alt-↓` cycles through a multi-call queue.
- ✅ With `decisionPrompt.enabled: false`, today's
  hang-on-request behaviour is restored (escape hatch).

## 13. Cross-references

- agentmuxai/agentmux#551 — parent issue (this spec extends it)
- `frontend/app/view/agent/buildRuntimeArgs.ts:18-24` —
  pre-turn PermissionMode → flag mapping
- `frontend/app/view/agent/types.ts` — `ToolCallEvent`
  definition to extend
- `frontend/app/platform/pane-overlay.ts` — `usePaneOverlay`
  reused for the decision panel's airspace
- `frontend/app/element/modal-v2.tsx` — ConfirmModal preset
  for the high-risk shift-click confirmation (when used)
- `agentmux-srv/src/server/websocket.rs` — where
  `tool:decision` RPC handler lands
