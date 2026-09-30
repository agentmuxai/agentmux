# REPORT: making the model/effort menu match what the agent actually runs

**Date:** 2026-09-30
**Status:** analysis — recommendations for a follow-up fix; not yet implemented
**Author:** AgentX (narko), at the owner's request
**Prompted by:** `docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md`.
The menu read "Sonnet 5.5 · high" while the agent ran Opus 5.5 at medium.
**Goal:** the model and effort shown in the menu are always what the agent is
actually using. Where the two can differ, the menu says so instead of showing a
selection.

## 1. The requirement, stated precisely

The pane has two different things to show, and today it shows only one:

- **Desired:** what the user selected (`agent:runtime` in block meta).
- **Effective:** what the running CLI is using right now.

The current bug is that the strip renders *desired* as if it were *effective*.
It was wrong here because the desired value never reached the process.

The rule this report proposes: **the strip shows effective state, read back
from the CLI.** A difference between desired and effective is a visible,
temporary state, not something the strip covers over.

## 2. What the sources say

### 2.1 Claude Code's own model and effort semantics
From the Model configuration docs:

- **Defaults, which explain the incident exactly.** On Pro/Max/Team/Enterprise
  and the Anthropic API, from v2.1.280 the default model is **Opus 5.5**. Opus
  5.5 and Sonnet 5.5 default to **medium** effort. A process given no `--model`
  or `--effort` therefore runs Opus 5.5 at medium. That's what AgentX got.
- **Aliases resolve per provider and change over time.** On the Anthropic API,
  `sonnet` resolves to **Sonnet 5.5** and `opus` to Opus 5.5. The same alias
  resolves differently on Bedrock, Vertex, Foundry and Claude Platform on AWS.
  So a label derived from our own catalog can't be trusted as what the alias
  resolves to. Only the CLI knows.
- **The priority order** for model: `/model` mid-session > `--model` >
  `ANTHROPIC_MODEL` > the `model` setting. For effort: `--effort` /
  `CLAUDE_CODE_EFFORT_LEVEL` / `effortLevel` and per-model `modelSettings`. A
  value we pass can be overridden by something higher, and a missing value
  falls through to a default that depends on the model.
- **In stream-json the remap warning is suppressed.** The docs say: "read the
  actual model from the `modelUsage` field of the result message instead." Take
  this narrowly — it's Anthropic's advice for that one suppressed-warning case,
  not a general "diff `modelUsage` against the effective model" recipe; see
  §3 item 6 for why the general version false-positives on subagents.

### 2.2 The CLI can be changed and read back live, without a respawn
From the Agent SDK TypeScript reference, and confirmed by reading the control
schema in the installed `claude.exe` 2.1.280:

| Control request (stdin, `type: control_request`) | What it does |
|---|---|
| `{subtype: "set_model", model}` | "Sets the model to use for subsequent conversation turns." `null` or `"default"` resets it. Since v2.1.212 a mid-turn switch applies from the next model call. |
| `{subtype: "apply_flag_settings", settings: {effortLevel}}` | Shallow-merges into the session flag layer. `effortLevel` applies on the next turn. This is the only path that accepts `max`. |
| `{subtype: "set_permission_mode", mode}` | Permission mode, live. |
| `{subtype: "get_settings"}` | "Returns the effective merged settings and the raw per-source settings". The response includes **`applied: {model, effort}`**: "runtime-resolved values after env overrides, session state, and model-specific defaults". |

`get_settings.applied` is the authoritative readback. The SDK docs note there
is no other getter: `initializationResult()` is a startup snapshot, and
`modelUsage` only looks back at a finished turn. AgentMux already talks this
control protocol to persistent agents (`--permission-prompt-tool stdio`).

Also useful:
- The `system`/`init` event carries `model` and `permissionMode`.
- Every `result` carries `modelUsage`, a per-model cost/usage breakdown — the
  CLI's own internal description of the field is "subagents started through
  the Agent tool in this session, as running totals". It is not evidence of
  which model the *session itself* ran on; see §3 item 6.
- Per a project testing against 2.1.283, `init` is emitted only after the first
  user message, so it can't be the only source.

### 2.3 Desired against observed state: the reconciliation pattern
The standard pattern from Kubernetes controllers, per the Kubebuilder good
practices and API conventions:

- **spec/status split.** The user writes desired state (`spec`). The controller
  observes the real world and publishes it (`status`). Neither pretends to be
  the other.
- **generation / observedGeneration.** Every change to desired state bumps a
  counter. The controller records which generation it last *applied and
  observed*. While `observedGeneration < generation` the status is stale.
  Tools like `kubectl wait` refuse to treat stale status as settled.
- **Level-triggered and idempotent.** Reconcile asks "what is different right
  now?", not "what event just happened?". Our bug is an edge-triggered design:
  the flags were only applied on the *send* event, so a spawn that happened
  without a send never got them.
- **Conditions.** Report `Progressing` / `Ready` / `Degraded` explicitly rather
  than implying success.

### 2.4 The same bug in another product
kamp-us/phoenix #8213, "Claude composer shows a selected effort level nobody
selected":

- **The bug:** the agent never read the session's running effort, so the
  bridge sent `current: null`, and the UI filled in the first level as if it
  had been picked.
- **The principle they adopted:** "effective values should be plainly
  distinguishable from requested or assumed values". "Unknown" must be a
  visible state, never a plausible guess.
- Their sibling issue #8062 is a picker left with no setter behind it.

## 3. What this means for AgentMux

1. **One reconciler, in srv, owned by the persistent controller.** Replace
   "apply flags when a message is sent" (frontend, edge-triggered) with a
   level-triggered check the controller runs at every point where it can act:
   - after spawn, including eager resume;
   - before delivering each message;
   - whenever `agent:runtime` changes.

   Each time it compares desired (`agent:runtime`) with effective (last
   readback). If they differ, it sends `set_model` / `apply_flag_settings` /
   `set_permission_mode`, then `get_settings` to confirm. This needs no
   respawn, doesn't interrupt a turn, and works no matter which launch path
   created the process (UI launch, continuation, MCP `OpenAgent`, relocation).
2. **Keep spawn flags as a first-pass hint, not the mechanism.** Still pass
   `--model` / `--effort` at spawn (fix bug 1 of the retro) so the first turn
   starts right. Correctness no longer depends on it, though: the reconciler
   fixes any spawn whose flags were missing.
3. **Generation counter.** Stamp `agent:runtime` with a `generation`, bumped
   on every user change. The controller records `observed_generation` plus the
   `applied` readback in controller status. The frontend compares the two.
4. **What the strip shows**, driven by the controller's effective state:

   | Situation | Strip shows |
   |---|---|
   | Readback matches desired | the value, e.g. `Sonnet 5.5 · high` (resolved id in the tooltip, e.g. `claude-sonnet-5-5`) |
   | Change pending (generation ahead of observed) | the new value marked as applying, e.g. `→ Sonnet 5.5 · high…` |
   | Readback differs from desired after reconcile | the **effective** value, with a warning: `Opus 5.5 · medium ⚠ selected Sonnet 5.5 · high` |
   | No readback yet (process not started, old CLI) | `Sonnet 5.5 · high (requested)`, never presented as confirmed |
   | Non-Claude providers with no readback | "requested" styling, per #8213's principle |

5. **Label from the CLI, not the catalog.** Take the display model from
   `get_settings.applied.model`, or `init.model` when that isn't available.
   The alias-derived label from the Models API overlay becomes the *picker's*
   option text only. That also settles "does `sonnet` mean 5.5 on this CLI":
   the readback answers it.
6. **`modelUsage` is not drift evidence, and don't use it as such.** Its own
   description in the CLI (`result.modelUsage`) is "subagents started through
   the Agent tool in this session, as running totals" — it is a per-model
   cost/usage breakdown, not an identity of "the model this turn ran on".
   An ordinary turn that delegates to a subagent legitimately reports more
   than one model in it (`docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md`
   §6 item 3 already flags this as unresolved), and the `result` message's
   own schema has no separate field naming the primary/session model — so a
   rule like "warn if `modelUsage` contains a model other than the effective
   one" would fire on every delegated turn, not just a real remap. **Drop
   this as a detection source entirely.** `get_settings.applied.model` (or
   `init.model`) is the only reliable ground truth this report found; treat
   `modelUsage` as cost/usage telemetry only, never as evidence of drift.
7. **Old CLIs.** If `get_settings` isn't supported (an error reply, or no
   `applied` in it), fall back to `init.model` for the model, and show effort
   as requested only.

## 4. Suggested order of work

1. **Retro fix 1, small and immediate:** `launchAgentDefinition` builds
   `cmd:args` with `buildRuntimeArgs`. It stops the Opus-by-default leak on
   continuations today.
2. **srv controller: readback.** Send `get_settings` after spawn and after
   each `result`, and publish `effective: {model, effort, permissionMode}` in
   controller status. Show it in the strip in its "requested" / "effective" /
   "mismatch" states.
3. **srv controller: reconcile.** Before each delivered message, apply the diff
   with `set_model` / `apply_flag_settings` / `set_permission_mode`, then read
   back. `runtime-apply.ts` stops needing a force-restart for model/effort
   changes.
4. **Generation counter** on `agent:runtime` and `observed_generation` in
   status.
5. **Tests:**
   - a fake CLI that ignores `--model` → the reconciler corrects it and the
     strip goes from mismatch to match;
   - a CLI that never answers `get_settings` → the strip shows "requested";
   - a continuation spawn → effective equals desired before the first reply;
   - a turn whose `modelUsage` includes a second (subagent) model → no
     warning, confirming §3 item 6's rule is actually followed and not just
     stated.

## 5. Sources

- Claude Code docs: Model configuration: https://code.claude.com/docs/en/model-config
- Claude Code docs: Agent SDK reference, TypeScript (`setModel`, `applyFlagSettings`, `initializationResult`, `modelUsage`): https://code.claude.com/docs/en/agent-sdk/typescript
- Control schema strings in the installed CLI (`claude.exe` 2.1.280): `set_model`, `apply_flag_settings`, `get_settings` with `applied: {model, effort}`; the `result` message's own schema (no top-level primary-model field) and its `modelUsage` field's internal description ("Subagents started through the Agent tool in this session, as running totals")
- `docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md` §6 item 3, flagging `modelUsage`'s multi-model-subagent shape as an open question independently of this report
- Kubebuilder book: Good Practices: https://book.kubebuilder.io/reference/good-practices.html
- Kubernetes status and conditions (observedGeneration, `kubectl wait`): https://www.golinuxcloud.com/kubernetes-status-and-conditions/
- KEP-5067, Pod generation / observedGeneration: https://github.com/kubernetes/enhancements/tree/master/keps/sig-node/5067-pod-generation
- kamp-us/phoenix #8213, effort shown that nobody selected: https://github.com/kamp-us/phoenix/issues/8213
- kamp-us/phoenix #8062, an effort picker with no setter: https://github.com/kamp-us/phoenix/issues/8062
- stream-json `init` timing, tested against 2.1.283: https://github.com/olesho/harness-wrapper/pull/90
