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
node ~/.agentmux/shell/muxspect.mjs find <block_id_or_agent>
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

## Finding which instance has a block/agent (`find`)

`find <block_id_or_agent>` answers "which running instance(s), if any, have
a controller or subagent dispatch matching this" — checks this instance
first, then every other channel via the shared reactive registry, with a
forwarded lookup only for a channel that actually matches (no network calls
wasted on channels that don't). A UUID-shaped argument is sent as a
`block_id` query; anything else as an `agent` name query.

```bash
muxspect find 71a6b2ae-b651-43aa-aed4-6121f24fd713
muxspect find Korp
```

An empty result is a legitimate answer — "not found on this host, in any
known channel" — not an error. Same LAN/WAN boundary as `conversations`
above (host + cross-channel only). See
`docs/specs/SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md`.

## Inspecting the pane layout (`layout`)

```bash
node ~/.agentmux/shell/muxspect.mjs layout            # every tab
node ~/.agentmux/shell/muxspect.mjs layout <tab_id>   # one tab
node ~/.agentmux/shell/muxspect.mjs layout --json     # machine-readable
```

Prints the layout tree as an indented outline — each node's direction, flex
size, block id, and whether it is minimized or magnified — followed by the
layout doctor's verdict.

```
tab tab1  (4 pane(s), 2 minimized)  healthy
  column (3)  size=10
    leaf dbe65c4c  size=10
    row (2)  size=10  [all-minimized]
      leaf 44807f01  size=10  [minimized]
      leaf 04cc1750  size=10  [minimized]
    leaf dd598cf3  size=10
```

**`[all-minimized]` on a branch is the one non-obvious flag.** A branch never
carries a `minimized` marker of its own; it is *effectively* minimized when
every leaf beneath it is, and that derived state — not the raw flag — is what
drives chip geometry. It was the distinction behind three pane-minimize bugs
(#2848, #2850, #2855), and it is invisible if you only read the stored flags.

Exits non-zero when any tree has invariant violations, so it can gate a script.

### Two caveats worth knowing

**It reports what is PERSISTED, not what is on screen.** The frontend derives
minimize geometry fresh on every render pass and only writes structural
changes back, so a tree here can lag what you are looking at. For "why does
this pane look wrong right now", this tells you the structure; it does not
tell you the rendered rects.

**The doctor runs against the tree on disk.** `validate_layout_invariants`
normally runs at reducer write-time, so violations here mean corruption that
is already persisted — including in trees written by older builds that never
ran the check. That is the case this command catches which nothing else does.

## Inspecting the work queue (`work`)

```bash
muxspect work              # every item, any state
muxspect work open         # just the claimable backlog
muxspect work claimed      # what is being worked right now, and by whom
muxspect work --json
```

Muxqueue is the shared backlog any agent can add to and any agent can claim —
the pull counterpart to jekt's addressed push delivery. See
`docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md`.

Each item shows its state, current holder, attempts burned against its limit,
any kind/target restriction, and the `result` field — which is the completion
trace on a `done` item and the reason on a `failed` or handed-back one.

**The thing to look for: `LEASE EXPIRED`.** A claimed item whose lease has
lapsed has no one working it, but nothing surfaces that on its own — the row
still says `claimed`, and it only returns to the pool the next time some agent
calls `WorkClaim` (reaping happens on claim; there is no background sweeper).
So a queue that looks busy can be entirely idle. This command flags those rows
explicitly and totals them at the end, rather than making you compare epoch
timestamps by eye.

Read-only, like the rest of `muxspect` — it never claims, completes, or cancels
anything. Use the `Work*` MCP tools for that.

**Scope note:** unlike everything else here, the queue lives in the
always-global identity store, so `muxspect work` shows items enqueued from
*any* channel on this machine, not just the instance you are inside. That is
deliberate — a per-channel queue would defeat the point — but it does mean this
one command is not subject to the single-instance limitation described below.

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

## Every response carries the instance's own version

Every `muxspect` call prints `[srv vX.Y.Z]` to stderr — the running
instance's own version, read from the `x-agentmux-srv-version` response
header every route sets. A 404 specifically gets a version hint folded
into the failure message: a stale local build predating whatever command
you just ran looks exactly like a missing route otherwise (a real gap this
session hit live — `muxspect conversations` 404ing against an
un-rebuilt instance). See `docs/specs/SPEC_MUXSPECT_SRV_VERSION_HEADER_2026_08_22.md`.
