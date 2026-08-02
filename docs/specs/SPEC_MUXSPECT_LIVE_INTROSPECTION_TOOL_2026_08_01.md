# SPEC — "muxspect": a live-state introspection tool for running AgentMux instances

**Date:** 2026-08-01
**Type:** New tool / subsystem design spec
**Trigger:** Live debugging session — the user asked to locate "2 stuck sleep
dock items," and neither `muxlog` nor any existing RPC could answer "what is
this running instance doing right now, and which instance is even hosting
this conversation." That gap is the direct motivation for this spec.
**Status:** Proposed — research complete, phased design below, no code yet.
**Related:** `docs/specs/REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`
(names the same six-plus overlapping liveness mechanisms this tool would read
from, not add an seventh to), `agentmux-srv/src/broker/process.rs` (Process
Broker Phase A — the read-side consolidation this tool builds directly on),
Discussion #2375 (process/turn-liveness tracking thread).

---

## 1. Why `muxlog` doesn't cover this

`muxlog` answers "what happened" — it walks NDJSON log files on disk across
every discoverable channel/dev-branch directory and never contacts a running
process. It has no concept of "current state": no live block list, no
"what's this agent doing at this exact moment," and critically, **no way to
even identify which running instance a given conversation or pane belongs
to** — the exact question that stalled the live debugging session this spec
grew out of. `muxspect` (working name — see §6 for alternatives) is the
missing live-query counterpart: point it at a running instance (or ask it to
discover all of them) and get a structured, drillable snapshot of what's
actually happening right now.

## 2. What already exists (full audit — see appendix references)

A thorough audit of the current backend found real, usable pieces — this is
explicitly **not** a green-field build:

- **`ProcessBroker`** (`agentmux-srv/src/broker/process.rs`) already computes
  a rich, cached, single-flight-guarded per-block `ProcessStatus`
  (`lifecycle`, `processes`, `liveness_confidence`, `controller_type`,
  `is_agent_pane`, `last_computed_ms`) via `status()`/`list()`. **But only
  `list_agent_panes()` is exposed via RPC today** (`agent.tracked-blocks`),
  and it returns bare `block_ids: Vec<String>` — every other field is
  computed and then thrown away before it reaches the wire. `status()` (the
  rich single-block read) has **zero RPC callers** anywhere in the codebase.
- `BlockControllerRuntimeStatus` (coarse lifecycle + `turn_active`) is
  reachable via `block.GetControllerStatus`, one block at a time.
- OS-process detail is reachable via `agent.process-list`, one block at a
  time, from a *different* registry (`AgentProcessRegistry`) than
  `ProcessBroker` itself reads.
- Subagent dispatch state is reachable via `subagent.ListActive`/`GetInfo`.
- **No RPC composes any of the above into one "describe this block" answer.**
  Getting a full picture of one block today means 3-4 separate, uncomposed
  RPC round-trips — the exact gap `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`
  names directly (§5.4, "the missing batch query").
- **The Activity Dock is not queryable at all, by anything, ever, today.**
  It's purely derived SolidJS state inside one renderer's own document store
  — never persisted, never sent over any RPC. This is a hard architectural
  boundary, not an oversight to "just expose" — see §5.3.
- **Zero cross-instance query mechanism exists.** Every instance's RPC
  surface (the WshRpcEngine, `ProcessBroker`, everything above) is reachable
  only by that instance's own frontend renderer, over its own local
  WS/HTTP. The *only* real cross-instance IPC today is (a) a bare
  liveness ping (`other_instances.rs`'s `probe_liveness` — connect, no data
  exchange, log-only) and (b) the authenticated `open_new_window` forward
  (bearer-token HTTP POST, explicitly side-effect-free by invariant I4).
  Discovery itself (`muxlog`'s own mechanism) is 100% disk-walking — it
  never asks a running process anything.

## 3. Best-practice synthesis (full research in the appendix)

Surveyed `kubectl`, `docker`, `systemctl`, Erlang/OTP `observer`/`recon`,
Chrome DevTools Protocol, `pm2`, `gh run`, Temporal's CLI, `htop`/Process
Explorer. Recurring principles, in priority order for this design:

1. **Read-only by default; mutation is always a separate, explicit action.**
   `htop --readonly`, `recon:info/1` deliberately dropping the mailbox field
   to stay production-safe, CDP's GET-only discovery endpoint. `muxspect`
   v1 has **zero mutating commands** — no kill, no restart. Those already
   exist elsewhere (`kill_tree`/`kill_pid`); this tool doesn't touch them.
2. **Three-verb shape: list → describe → watch.** Every mature precedent
   converges on this (`kubectl get`→`describe`→`-w`; `docker ps`→`inspect`→
   `stats`; `pm2 list`→`describe`→`monit`; Temporal `list`→`describe`→
   `show --follow`). §4 adopts this directly.
3. **Discovery-as-a-first-step, separate from targeting.** `kubectl config
   get-contexts`, `docker context ls`, CDP's `/json`, `epmd -names` all
   separate "what are my options" from "operate on option X." `muxspect`'s
   own multi-instance gap (§2) makes this the *load-bearing* command, not a
   nicety — see `muxspect targets` in §4.
4. **Structured output opt-in, human-readable default.** `--json` everywhere
   surveyed follows this split; `muxspect` should too.
5. **Snapshot vs. watch as one flag, not a separate mental model.** `docker
   stats --no-stream`, `kubectl get -w`, `gh run watch`.
6. **"Explain why," not just "what."** `kubectl describe`'s Events section
   and Temporal's Event History are the standout precedent — a causal log
   distinct from both the summary and the point-in-time detail view.
   `ProcessBroker` already emits `processbroker:status-changed`; `describe`
   should surface a short recent-events tail, not just the current snapshot
   (§4, Phase 3 — deliberately not v1).
7. **Surface staleness/confidence, never hide it.** `ProcessBroker` already
   carries `last_computed_ms` and `liveness_confidence` — good, this is
   exactly the discipline the research recommends, and it's already in the
   data model. `muxspect` must always print these, not just the raw state.
8. **Anti-pattern to actively avoid: don't become an eighth mechanism.** The
   AWS S3 dashboard outage (its own status page was hosted on the service it
   reported on, so it stayed green through the outage) and the GitHub 2018
   incident (a topology tool acted on cached state that had silently
   diverged from ground truth) are the two sharpest cautionary precedents.
   **`muxspect` must be a thin, read-only client over `ProcessBroker`'s
   existing computation — never a second, independent state-tracker that
   can drift from it.** This is the single most important constraint on the
   whole design; see §5.1.

## 4. Proposed command shape

```
muxspect                                  # describe the CURRENT instance (env-sourced, see §5.2)
muxspect list [--json]                    # summary table, current instance
muxspect describe <block_id> [--json]
muxspect watch <block_id>                 # live-updating describe
muxspect targets                          # (Phase 2) discover every OTHER running instance
muxspect list -i <target> / describe -i <target>   # (Phase 2) query a different instance
```

- **`muxspect list`** — a summary table (block_id, controller_type,
  lifecycle, turn_active, confidence, last_computed_ms) for the current
  instance. Requires exposing `ProcessBroker::list()`'s **full**
  `ProcessStatus`, not just block_ids — a **new** route (not a breaking
  change to `agent.tracked-blocks`, which the Swarm pane already depends on
  for its specific narrower shape) — added under a clearly diagnostic-only
  path (e.g. `/api/v1/muxspect/list`) to keep it obviously separate from
  product-facing routes.
- **`muxspect describe <block_id>`** — the new composition point: one route
  that calls `ProcessBroker::status()` + `BlockControllerRuntimeStatus` +
  `AgentProcessRegistry::list_block()` + (if applicable)
  `subagent.GetInfo`, and returns them together. This is the single query
  `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` §5.4 already
  named as missing — `muxspect describe` is its first real consumer.
- **`muxspect watch`** — same payload as `describe`, polling or subscribing
  to `processbroker:status-changed` for that block scope, printed as a
  scrolling diff (matching `docker stats`/`kubectl get -w`'s "same command,
  one flag" pattern rather than a separate tool).
- **`muxspect targets`** — Phase 2 only (§7); enumerates *other* running
  instances via `muxlog`'s own disk-walk + liveness probe.

## 5. Design decisions that need explicit resolution

### 5.1 Build on `ProcessBroker`, don't fork it

Every new route this tool needs is a **read composition over `ProcessBroker`
and its existing sibling registries** — never a new independent snapshot of
process/turn state. This is non-negotiable per §3 point 8. Concretely:
`muxspect list`/`describe` call into the *same* `ProcessBroker::status()`/
`list()` the app's own Swarm pane will eventually call once Process Broker
Phase B lands (tracked separately, Discussion #2375) — this tool and the
in-app UI converge on one source of truth rather than growing two.

### 5.2 Connecting to the current instance — reuse the existing HTTP+token path, no new IPC

**Corrected during design (2026-08-01):** the first draft of this section
proposed a brand-new authenticated named-pipe IPC command for querying
"the current instance." That's unnecessary — verified directly, there's
already a real, working precedent for exactly this: **`agentmux-mcp`**, a
standalone binary spawned as a child of an agent CLI, talks to `agentmux-srv`
over plain HTTP (`POST {AGENTMUX_LOCAL_URL}/api/v1/...` with header
`X-AuthKey: {AGENTMUX_AUTH_KEY}`, `agentmux-mcp/src/main.rs`), reading both
values from its own **inherited process environment** — no file, no new
discovery mechanism.

Confirmed directly in this session's own shell (an agent-pane process):
`AGENTMUX_LOCAL_URL` and `AGENTMUX_AUTH_KEY` are already present in the
environment. `shell/lifecycle.rs` propagates `AGENTMUX_LOCAL_URL` to every
plain shell/terminal pane too; `AGENTMUX_AUTH_KEY` specifically is injected
by `agent_handlers/input.rs` for agent-CLI-type controller spawns (matching
`agentmux-mcp`'s own environment). So: **`muxspect`, run from within an agent
pane (the primary motivating use case — an agent inspecting its own running
instance), already has everything it needs, today, with zero new plumbing.**

Design: `muxspect list`/`describe`/`watch` read `$AGENTMUX_LOCAL_URL` /
`$AGENTMUX_AUTH_KEY` from their own environment and call the two new HTTP
routes (§4) the exact same way `agentmux-mcp` already calls existing ones.
If invoked somewhere those env vars aren't set (a plain, non-agent shell
pane invoked outside that propagation path, or a genuinely external
terminal), it must fail with a clear, honest error — never guess, never
silently fall back to a different (possibly wrong) instance. This is a
direct application of §3 point 8 (never paper over unreachability) and
means Phase 1 requires **no new IPC mechanism, no new auth scheme, and no
named-pipe changes** — only two new authenticated HTTP routes reusing
`auth_middleware` exactly as every other route already does.

Cross-instance query (asking a *different* running instance, not the one
you're already inside) is a genuinely separate problem — deferred to Phase 2
(§7), since it requires a real discovery+auth story for instances whose
token you don't already have in your environment (the dev-only `authkey.dev`
file is the closest existing precedent, but it's explicitly dev-gated).

### 5.3 Activity Dock visibility — explicitly out of scope for v1, with a named escape hatch

The Dock is pure in-renderer SolidJS state (§2) — no backend-only tool can
ever see it without either (a) a real architecture change (mirroring
`ToolNode` state server-side, a much bigger lift, likely overlapping the
"shared orphan-recovery invariant" work already tracked as a separate docket
item) or (b) browser-level remote debugging. **AgentMux already ships CEF**,
which supports the same `--remote-debugging-port` Chrome DevTools Protocol
surveyed in §3 — the existing, off-the-shelf answer for "inspect this
renderer's live JS state" is to enable and document that port, not build a
custom mirror. Recommendation: `muxspect targets` can opportunistically print
each instance's devtools URL if the port is easily discoverable per-channel,
as a low-effort bonus — but full Dock introspection is explicitly **not** a
`muxspect` v1 goal. Document this limitation loudly so nobody re-derives "why
can't muxspect see the dock" a second time.

### 5.4 Staleness and failure modes

Per §3 point 7/8: every `muxspect` output must show `last_computed_ms` /
`liveness_confidence` (already in `ProcessStatus`) front and center, and
every cross-instance query must clearly distinguish "instance unreachable"
from "instance reachable, block not found" from "instance reachable, block
exists, state X" — collapsing these into one ambiguous "unknown" is exactly
the watermelon-dashboard failure mode §3 point 8 warns against.

## 6. Name candidates

- **`muxspect`** (working title used throughout this spec) — "inspect,"
  reads naturally alongside `muxlog`, distinct verb class (log vs. inspect).
- `muxstat` — shorter, echoes `stat`/`systemctl status`, but weaker signal
  that it's live (could be misread as one-shot like `docker system info`).
- `muxtop` — evokes the `htop`/`docker stats` continuously-live precedent
  well, but undersells the `describe`/drill-down half of the tool.
- `muxprobe` — emphasizes the cross-instance liveness-probe angle (§5.2)
  specifically, less clear on the local single-instance `list`/`describe`
  use case.

No strong reason to deviate from `muxspect` unless there's a naming
collision or preference — recommend it as primary, open to the others.

## 7. Phased plan (no big-bang, matching this repo's own established pattern)

1. **Phase 1 — current instance only, no new IPC.** `muxspect list`/
   `describe`/`watch` against whatever instance the caller's own
   `$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY` point at (§5.2) — exactly two
   new authenticated HTTP routes on the existing `web_listener`
   (full-detail list + the describe-composition endpoint, §4), reusing
   `auth_middleware` unchanged. This is genuinely small: no new auth scheme,
   no new IPC, no discovery mechanism — the primary motivating use case (an
   agent inspecting its own running instance) is fully served by this phase
   alone.
2. **Phase 2 — cross-instance query.** `muxspect targets` (disk-walk +
   existing liveness probe, reusing `muxlog`'s own mechanism, zero backend
   changes) plus a real discovery+auth story for reaching an instance whose
   token isn't already in the caller's environment — the dev-only
   `authkey.dev` file is the closest existing precedent but is explicitly
   dev-gated; a production-viable equivalent (or an explicit "dev builds
   only" scope limit) needs a decision before this phase starts.
3. **Phase 3 — optional, lower priority.** A short recent-events tail for
   `describe` (§3 point 6) — needs a small bounded ring buffer per block if
   one doesn't already exist; devtools-URL surfacing in `muxspect targets`
   (§5.3) as a convenience, not a requirement.

**Explicitly not in scope, any phase:** any mutating action (kill/restart —
already exists elsewhere, this tool doesn't duplicate it), and full Activity
Dock/renderer-state introspection (§5.3 — CEF's own devtools protocol is the
right tool for that, not a `muxspect` feature).

### 7.1 Known gap: the shell function is unreachable exactly where it matters most (reagent P1 on PR #2380)

Discovered during review, confirmed empirically (`type muxspect` → not found
in an actual agent tool-call shell): the bare `muxspect` shell function only
loads in an **interactive** terminal pane (sourced from the rcfile at PTY
spawn) — but per §5.2's own research, those panes carry
`$AGENTMUX_LOCAL_URL` WITHOUT `$AGENTMUX_AUTH_KEY` (only agent-CLI-type
controller spawns get the key, via `agent_handlers/input.rs`). Agent tool
calls (this session's own primary motivating use case) get both env vars,
but tool-spawned shells (`bash -c "..."` style, no `--rcfile`) don't source
the integration script at all, so the function isn't defined there either.
Net result: the convenient `muxspect ...` invocation doesn't actually work
in EITHER context today — only the fully-qualified
`node ~/.agentmux/shell/muxspect.mjs ...` (which this PR verified works end
to end) does.

**Fix deferred, not attempted in this PR** — two candidates, both requiring
more careful, separate scoping than a drive-by fixup:

1. Wire a `muxspect` launcher into whatever mechanism already adds
   AgentMux-managed tool dirs to an agent-CLI subprocess's `PATH` (see
   `shell/lifecycle.rs`'s "Wire AgentMux-managed tool dirs into the agent's
   PATH" comment) — makes bare `muxspect` callable exactly where the auth
   key already exists, no new env propagation needed.
2. Propagate `$AGENTMUX_AUTH_KEY` into interactive shell panes too (not
   just agent-CLI spawns) — **not recommended without independent
   security review**: that key currently authorizes every `/api/v1/*`
   route, not just the two read-only `muxspect` ones, so widening its
   propagation to every plain terminal pane is a real scope-of-access
   change, not a documentation fix.

Until one of these lands, `docs/MUXSPECT.md`, `CLAUDE.md`, and
`muxspect.mjs`'s own `--help` text all point users/agents at the
direct-path invocation instead of overselling shell-function convenience
that doesn't apply yet.

**Follow-up finding, same review (codex P1, next round):** the direct-path
fallback above isn't reliably available either — `deploy_scripts` (which
writes `muxlog.mjs`/`muxspect.mjs` to `<data_dir>/shell/`) had exactly one
runtime call site, `ShellController::start`'s interactive (empty-command)
branch — a user whose first-ever pane in a fresh data dir is an Agent pane
(persistent/subprocess/acp, which never takes that branch) would never get
either script deployed at all. A **pre-existing gap in `muxlog`'s own
deployment mechanism** this PR newly depends on, not something introduced
here. **Fixed**: `deploy_scripts` is now also called unconditionally at srv
startup (`bootstrap.rs::spawn_background_subsystems`), before any block can
be created — idempotent (skips if its version marker already matches), so
this is additive, not a behavior change to the existing call site. The
direct-path fallback (§7.1's own recommendation above) is now always
available regardless of what pane type the user opens first; the shell
*function*'s unreachability (the original finding) is unchanged and still
needs one of the two candidates above.

---

## 8. Decisions (resolved 2026-08-01)

1. **Name: `muxspect`.** Confirmed.
2. **First PR scope: Phase 1 only** (two new HTTP routes + `muxspect list`/
   `describe`/`watch` against the current instance), holding Phase 2/3 for
   later — matching this session's own established pattern of small,
   incremental, reviewed PRs.
3. **Location: a true sibling of `muxlog`**, not a new Rust binary.
   `muxlog`'s actual deployment mechanism (traced directly, not assumed):
   its Node source lives at `agentmux-srv/src/backend/shellintegration/muxlog.mjs`,
   embedded into the srv binary via `include_str!` in `shellintegration.rs`,
   and deployed once per version to `<wave_data_dir>/shell/muxlog.mjs` by
   `deploy_scripts()` — each of the four per-shell integration scripts
   (`bash.sh`/`zsh.sh`/`pwsh.ps1`/`fish.fish`) defines a `muxlog` function
   that resolves that path relative to itself and delegates to
   `node muxlog.mjs "$@"`. `muxspect` follows the identical pattern: a new
   `agentmux-srv/src/backend/shellintegration/muxspect.mjs`, a new
   `MUXSPECT_JS` `include_str!` constant, deployed alongside `muxlog.mjs` in
   the same `deploy_scripts()` pass, with a matching `muxspect` function
   added to each of the four shell scripts. Unlike `muxlog`, there is no
   legacy non-Node fallback to preserve — introspection requires an actual
   authenticated HTTP call, so "Node unavailable" is just a clear error, not
   a degraded-but-working path.

---

*Research complete (external best-practices + full internal API audit, both
via background agents). No code changes — design/scoping only.*
