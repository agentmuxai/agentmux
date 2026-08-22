# `muxspect` — live process/turn-state introspection

`muxspect` is `muxlog`'s live-state sibling: `muxlog` answers "what
happened" (historical NDJSON logs); `muxspect` answers "what is this
instance doing **right now**." Shipped the same way, in every AgentMux
terminal (bash / zsh / pwsh / fish), delegating to a small Node core
(`muxspect.mjs`, deployed next to `muxlog.mjs`).

> **Phase 1 scope:** `muxspect` only queries the instance you're already
> inside — it reads `$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY` from its own
> environment (already present in any agent pane, the same way
> `agentmux-mcp` reaches every other `/api/v1/*` route). It cannot yet
> discover or query a *different* running instance — see
> `docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` for the
> full design and the planned Phase 2.

---

## Quick start

> **Known gap (reagent P1 on PR #2380, not yet fixed):** the bare `muxspect`
> shell function only loads in an *interactive* terminal pane (sourced from
> the shell rcfile) — but those panes carry `$AGENTMUX_LOCAL_URL` without
> `$AGENTMUX_AUTH_KEY`, so the function loads but auth fails. Agent-CLI tool
> calls (the primary intended use) get both env vars, but tool-spawned
> shells don't source the rcfile, so the function isn't defined there
> either — empirically confirmed (`type muxspect` → not found) in an actual
> agent tool-call shell. **Until this is fixed, call the deployed core
> directly instead of the shell function:**

```bash
node ~/.agentmux/shell/muxspect.mjs list
node ~/.agentmux/shell/muxspect.mjs describe <block_id>
node ~/.agentmux/shell/muxspect.mjs watch <block_id>
node ~/.agentmux/shell/muxspect.mjs dock <block_id>
node ~/.agentmux/shell/muxspect.mjs dock clear <block_id> <node_id>
node ~/.agentmux/shell/muxspect.mjs conversations
node ~/.agentmux/shell/muxspect.mjs conversation <agent>
node ~/.agentmux/shell/muxspect.mjs help
```

Add `--json` to `list`/`describe`/`dock` for structured output. Once the
shell function actually works everywhere it's meant to, bare `muxspect ...`
will work too — the wire protocol and commands are identical either way.

## Diagnosing and clearing a stuck Activity Dock entry (`dock`)

`list`/`describe`/`watch` read `ProcessBroker`/controller state — they
cannot see the Agent pane's Activity Dock (see "What it can and can't
see" below): an in-renderer `ToolNode` that never received its
terminating event (e.g. a tool call rejected by the outer CLI harness
before it ever ran) can stay stuck at `status: "running"` in the dock
indefinitely, invisible to every other `muxspect` command.

`dock <block_id>` reads a lightweight snapshot the renderer itself pushes
on every `ToolNode` status change (id, tool name, status, age, whether it
was a `run_in_background` launch — no transcript content) and flags
entries that look stuck (`running`, past the 30s promotion threshold,
with nothing srv-side backing the block).
`dock clear <block_id> <node_id>` force-cancels one specific entry live,
in whatever renderer currently has that block open — no pane reload
needed. This is `muxspect`'s only mutating command; every other command
is read-only diagnostics.

The `STUCK?` heuristic has a blind spot for backgrounded launches (issue
#2518): an accepted `run_in_background` launch's raw `ToolNode.status`
goes terminal (`success`) within ~a second, so `STUCK?` — which only ever
fires on `status: "running"` — structurally can never flag one, no matter
how long the *actual* dock row has been showing `running` while it awaits
a `<task-notification>` (that reclassification is entirely client-side,
in `tool-adapter.ts`; the srv never sees it). The `bg` column is the one
signal that survives the trip: a `bg` row with an old `age` and
`status: success` is worth checking by hand in the live UI even though
`STUCK?` reads clean for it.

Full design: `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md`.
This targets one specific, narrow bug class — it is not a fix for the
underlying issue (a first-party in-renderer self-expiry fix is tracked
separately, see that spec §1.2) and not general Activity Dock
introspection (see "What it can and can't see" below, still accurate for
everything except this one diagnostic snapshot).

## Seeing every agent's conversation at a glance (`conversations` / `conversation`)

`conversations` lists every agent this instance can see — host (this
instance's own registry) and cross-channel (other AgentMux channels on
this same host) entries include `turn_active` and a last-message preview
(the tail non-blank transcript line); LAN and WAN entries are
liveness-only for now (`remote_fetch_required: true` — reading their
conversation content isn't supported yet). `conversation <agent>` reads
the actual transcript tail for one named agent, resolving host first,
then cross-channel — same underlying route `GetAgentTranscript` uses, so
an agent's own MCP tool call and a human running this command see
identical data.

This is Phase A of `docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
— it does not reach LAN or WAN conversation content (Phase B/C, not yet
built, and gated on an explicit repo-owner-confirmed addition to
`CLAUDE.md`'s jekt security rules before they can be).

## What it can and can't see

`muxspect` is a thin, read-only client over the same `ProcessBroker`
computation the app's own Swarm pane uses — it never invents a second,
independent view of process/turn state. Concretely it can see:

- Controller lifecycle (`Running`/`Idle`/`Done`/`Error`/`Unknown`) and
  whether a turn is actively in flight
- The OS process tree tracked for a block (PID, command, RSS)
- How stale each of the above is (`last_computed_ms`) and how confidently
  it's tracked on this platform (`liveness_confidence`)
- **`last_error`** — if the very last line ever written to a block's own
  transcript is a persisted `error_during_execution` frame (a spawn that
  was refused before any process/controller existed — a bad identity
  link, a container that wouldn't start, etc.), `describe` and `list` both
  surface it directly: the message, a best-effort `source` tag, and when
  it was written. This is what a block with `lifecycle: unknown` and zero
  processes actually means — "healthy, idle" and "permanently wedged,
  actionable fix available" look identical without it. Added to close
  exactly that gap after a real incident where `muxspect describe` came
  back diagnostically empty on a wedged agent — see
  `docs/reports/REPORT_MUXSPECT_SPAWN_REFUSAL_DIAGNOSIS_EXTENSION_2026_08_03.md`.
  Not a live/independent error tracker — it's a second reader over the
  same durable frame the pane's own error bubble already comes from, so it
  can never drift from what the UI shows.

It **cannot** see the Agent pane's Activity Dock — that's pure in-renderer
SolidJS state, never persisted or exposed via any RPC, by design (not an
oversight `muxspect` will "eventually" fix). For that, use the CEF host's
own remote-debugging port (Chrome DevTools Protocol) — see the spec's §5.3
for why that's the right tool for that specific gap.

## Why it can't just query any instance yet

Every AgentMux instance runs its own `agentmux-srv` on a dynamic port with
its own auth key (isolation invariants I1–I6) — there's no existing
mechanism for one instance to answer a state query from outside itself
except the path `muxspect` already uses (env-inherited token, same as
`agentmux-mcp`). Reaching a *different* instance needs a real
discovery+auth story that doesn't exist yet — planned as Phase 2.
