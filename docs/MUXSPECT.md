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
node ~/.agentmux/shell/muxspect.mjs help
```

Add `--json` to `list`/`describe` for structured output. Once the shell
function actually works everywhere it's meant to, bare `muxspect ...` will
work too — the wire protocol and commands are identical either way.

## What it can and can't see

`muxspect` is a thin, read-only client over the same `ProcessBroker`
computation the app's own Swarm pane uses — it never invents a second,
independent view of process/turn state. Concretely it can see:

- Controller lifecycle (`Running`/`Idle`/`Done`/`Error`/`Unknown`) and
  whether a turn is actively in flight
- The OS process tree tracked for a block (PID, command, RSS)
- How stale each of the above is (`last_computed_ms`) and how confidently
  it's tracked on this platform (`liveness_confidence`)

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
