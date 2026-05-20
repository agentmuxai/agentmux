# SPEC — Service Supervision & Recovery

**Status:** Draft / for design review
**Date:** 2026-05-20
**Author:** AgentA
**Tracking:** to get a long-lived GitHub Discussion thread (sibling of #707 for
the reducer stack)
**Case study:** `docs/analysis/CRASH_GPU_PROCESS_FATAL_2026_05_20.md`

---

## 1. Motivation — the stability mandate

Rock-solid stability is a core part of AgentMux's value proposition. The
standing product goal is **"no crashes ever."** Stated precisely, because the
precision is what makes it shippable:

> **"No crashes ever" = the user must never *see* a crash.**

Zero *process faults* is not achievable — GPU drivers segfault, the OS runs out
of commit, hardware misbehaves, and none of that is AgentMux's code. Zero
*visible* crashes **is** achievable: every fault becomes an invisible,
sub-second auto-recovery — no OS modal, no lost work, at most a flicker.

The 2026-05-20 host crash is case study #1. The Chromium GPU process failed
(environment: a flaky driver + an exhausted page file), Chromium `LOG(FATAL)`-ed
the whole host, and the user got a raw Windows `0x80000003` "breakpoint" modal.
A GPU subsystem failure — *not even AgentMux's own code* — took the entire app
down visibly. That is the exact class of event this framework must absorb.

AgentMux is unusually well-positioned to deliver the achievable guarantee: it
already has a separate supervising process (`agentmux-launcher`) and already
persists all application state. The recovery machinery is ~70% latent in the
architecture. This spec names it, unifies it, and closes the gaps.

---

## 2. Goals / Non-goals

### Goals
- **Uniform recovery *discipline*** across every long-lived service (launcher,
  host, srv, Chromium children, sidecar workers) — implemented as per-service
  managers that share primitives and obey one prime directive (§3, §4), **not**
  a single unified framework.
- Any service crash → **detect → restart → rehydrate → reconcile**, fast enough
  that the user sees at most a flicker.
- **Bounded** recovery — a crash budget and a graceful terminal state; never an
  infinite restart loop.
- **Near-zero steady-state cost** — see §9.
- Recovery paths **continuously fault-injection-tested** in CI (§14).
- Auto-recovery is **loud internally** — counted, surfaced, alarmed on (§12).

### Non-goals
- Catching a Chromium `LOG(FATAL)` or a Rust `panic = abort` *inside* the
  faulting process. These are synchronous and terminal by design. Recovery is
  always cross-process: a *supervisor* observes the death and acts.
- Eliminating process faults. We absorb them, we do not prevent the
  environment from causing them.
- A parallel snapshot/checkpoint system. Recovery rehydrates from the
  persistence that **already exists** (§6, §9).
- Replacing the reducer stack, the saga coordinator, or WRR. The framework
  *composes* with them (§13).

---

## 3. Prime directive

Two design concerns — performance overhead, and recovery systems that "shoot
themselves in the foot" — converge on one rule. It is the framework's prime
directive; a design that violates it is rejected at review:

> **The supervisor is passive, bounded, and simpler than everything it
> supervises.** It observes signals that *already exist*, free-rides on
> persistence that *already happens*, acts only on real crashes, is bounded by
> a crash budget, and is loud about every action it takes.

Everything below is an elaboration of this directive.

---

## 4. Core approach — per-service managers + shared primitives

**Decision: per-service managers, not a unified framework.** Each service gets
its *own* manager with its own crash/restart/rehydrate logic. Managers share
low-level *primitives*; they do not implement one uniform behavioral contract.

### Why — the VS Code precedent

VS Code is a mature, far larger multi-process app facing the same problem, and
it deliberately did **not** build a unified supervisor framework:

- **`UtilityProcess`** (`src/vs/platform/utilityProcess/electron-main/`) — a
  shared process *primitive*: spawn, MessagePort wiring, `onCrash` / `onExit`
  events. It *reports* crashes; it does not restart or rehydrate.
- **Extension host** — its own manager (`ExtensionHostManager` /
  `AbstractExtensionService`) with a crash counter: after ~3 crashes in a short
  window it stops auto-restarting and shows "Extension host terminated
  unexpectedly — Restart?" (= our crash budget + terminal state, §7 / §10-A).
- **Pty host** — `PtyHostService`: its own restart logic, terminal
  *reconnection* across a window reload (= our Tier-1 pattern, §6), and
  unresponsive-detection by ping (= our liveness + responsiveness, §8).
- **Shared process** — supervised ad-hoc by the Electron main process.

No `SupervisedService` trait, no uniform contract — each manager is bespoke,
because the services' recovery semantics genuinely differ. They differ here
too: a host crash is Tier 1 (re-attach to a live srv), an srv crash is Tier 2
(resume), the GPU has its own escalation. Forcing them through one contract
would be abstraction for its own sake — the over-reach the original draft of
this spec leaned toward.

### The model

- **Per-service managers.** `HostManager`, `SrvManager`, … — each owns one
  service's crash detection, retry ladder (§7), rehydration (§5), and lifecycle
  events. Bespoke, shaped to that service's tier and failure modes. Every
  manager still answers the same questions, just in its own way: *identity*
  (stable id + generation number for fencing, §10-D), *spawn*, *health*
  (liveness + responsiveness, §8), *rehydrate*, *retry ladder*, and
  *onCrash / onUp* events.
- **Shared primitives** in `agentmux-common`, used by the managers — extracted
  **only when ≥2 managers actually need them**, never speculatively:
  - a process-handle wrapper (spawn + OS exit/crash event — the `UtilityProcess`
    analog);
  - the crash-budget + backoff helper;
  - the transient/deterministic classifier (§7);
  - the recovery-metric counter (§12).
- **No uniform behavioral contract.** Managers are not forced into one shape.
  Generalization is bottom-up — extract a primitive when it demonstrably
  repeats — never top-down.

A restarted service is still **not a special case**: the manager re-attaches it
as a new subscriber that needs the current snapshot (§5). That simplification
holds either way.

---

## 5. Recovery = Restart + Rehydrate

Recovery has exactly two halves, owned by two different layers:

| Half | Owner | Source of truth |
|---|---|---|
| **Restart** — get a fresh process running | Supervisor (this spec) | — |
| **Rehydrate** — bring it to correct state | Reducer stack + persistence | `objects.db`, `sagas.db`, `launcher-events.log` |

This is the precise relationship to the reducer stack. The supervision
framework is the **dual** of the reducer/persistence layer, not part of it:

- Reducer stack — *what* the correct state is.
- Persistence — *where* that state is durable.
- **Supervision (new)** — *keeps every service alive and reconciled to it.*

The framework **consumes** the reducer as its source of truth. It never owns
state and never adds a write path (§9).

---

## 6. Layer & ownership map

| Service | Supervised by | Rehydrates from | Restart today |
|---|---|---|---|
| `agentmux-srv` (Layer 3) | launcher | `objects.db`, `filestore.db` | launcher spawns it; no auto-restart |
| `agentmux-cef` host (Layer 2) | launcher | `objects.db` (workspaces/tabs/panes/layout) | launcher spawns it; no auto-restart |
| GPU process | host → Chromium | re-attach GPU channels | **Chromium auto-restarts + re-mounts** (works; capped) |
| Renderer processes | host → Chromium | reload + frontend slices rehydrate | Chromium auto-reloads |
| Sidecar workers / sagas | saga coordinator | `sagas.db` | **already crash-resumes** |
| `agentmux-launcher` (Layer 1) | **root — see below** | `launcher-events.log` | single-instance re-attach only |

### The root-supervisor problem

You cannot recurse supervisors forever — something is the root. The launcher is
that root. The resolution is **not** another watchdog process by default; it is:

1. The launcher is kept **deliberately small, boring, and bulletproof** — the
   least code, the most tests, the fewest dependencies of any component.
2. The single-instance named pipe means a fresh launch **re-attaches** to
   surviving children rather than colliding.
3. **Optional** escalation if data shows the launcher itself ever crashes: a
   minimal OS-level respawn (Scheduled Task / Service) or a tiny watchdog whose
   only job is "is the launcher pid alive." This is an open decision (§15) —
   not built until evidence justifies it.

### Two recovery tiers — host crash vs srv crash

The process topology means recovery cost is **not uniform**. There are two
tiers, and which one a crash lands in is decided entirely by *which process
died* — verified against the code:

**Tier 1 — host crash (cheap).** `agentmux-srv` is a **sibling** of the host,
both spawned by the launcher (`agentmux-launcher/src/main.rs`: "launcher now
spawns srv directly (sibling of host)"). The agent CLI process and its PTY are
children of **srv**, not the host — `PersistentSubprocessController`
(`agentmux-srv/.../blockcontroller/persistent.rs`) keeps a single CLI process
alive for the whole session. So a host crash kills **only the UI**. srv, the
agent process, and the PTY are untouched; srv keeps streaming the agent's
stdout into the block's `.jsonl` and WPS `output` subject the whole time the
host is down. Recovery = the supervisor relaunches the host, which re-attaches
to the *still-running* srv, reloads the block tree + layout from `objects.db`,
and re-subscribes to each block's WPS `output` subject (`.jsonl` replayed).
**No special content logic** — content survival is a free consequence of the
topology. The live agent session never stopped.

**Tier 2 — srv crash / OS reboot / full kill (harder).** Here the agent
subprocess dies *with* srv (it is srv's child). The session **history** is
still durable in FileStore (`.jsonl`) — content is not lost — but the live
process is gone. `agentmux-srv/.../blockcontroller/session_recovery.rs` already
scaffolds graceful resume: a `session:active_pid` meta flag, flipped to
`session:was_interrupted` on the next boot, surfaces a banner, and the
persistent controller supports `--resume <session_id>` to pick the conversation
up mid-flight. So content persists, but the live process needs a (one-click)
**resume**, not silent continuation.

Implication for phasing: **host supervision (Phase 1) is the Tier-1 cheap case
and ships first** — content persistence is free, the supervisor only relaunches
and re-attaches. **srv supervision (Phase 2) is the Tier-2 case** and builds on
the existing `session_recovery.rs` resume path. The 2026-05-20 crash was a
Tier-1 host crash: srv and the agent process were alive behind the modal the
whole time; only an automatic host relaunch was missing.

---

## 7. Crash classification — transient vs deterministic

The single most dangerous mistake a recovery system makes is restarting into
the same crash forever. Before *every* restart, the supervisor classifies:

- **Transient** — a driver hiccup, an OOM spike, a one-off. Signature differs
  from recent crashes, or the environment changed. → restart normally.
- **Deterministic** — same crash signature N times in a row (same exit code +
  same fault location/log signature). Restarting reproduces it. → **do not
  restart with the same inputs** — step down the retry ladder.

### Retry ladder

The first retry is always **optimistic — full config, nothing degraded**. Most
crashes are transient (a driver hiccup, an OOM spike) and a full restart just
works; the supervisor must not permanently degrade the user for a one-off. The
supervisor steps down a rung **only when a restart reproduces the crash**:

| Rung | When | What |
|---|---|---|
| 1 | first crash (assumed transient) | restart **full config**, current state |
| 2 | rung 1 reproduced the crash | restart **degraded** (e.g. host `--disable-gpu`) |
| 3 | rung 2 reproduced it | restart from **last-known-good** state, not crash-time state |
| 4 | rung 3 reproduced it | **safe mode** — minimal state, recovery UI shown |
| 5 | budget exhausted | **terminal** — stop, honest recovery dialog, offer report/reset |

Each rung asks *less* of the environment than the one above, so a degraded or
safe-mode retry has a genuinely better chance than the original — it is not
"the same thing again." A crash signature =
`{ exit_code, faulting module/log line, service id }`.

---

## 8. Health signals

"Process exited" is not enough. The 2026-05-20 crash is the proof: the host
process stayed alive and heart-beating while the renderer was frozen.

Health = **two** independent signals:

- **Liveness** — is the OS process alive? Source: Job Object / process-exit
  handle. Event-driven, zero cost.
- **Responsiveness** — does the service answer a ping within a budget? Source:
  reuse the existing `mem_heartbeat` / launcher-pipe channel; add a cheap
  request/response.

Rules to avoid false-positive kills (§10-C): generous timeouts, **multiple**
consecutive missed pings before action, and never confuse *slow* (GC pause,
legit long op) with *dead*.

---

## 9. Performance budget — first-class

A design that cannot meet this budget is rejected.

**Steady-state target: ≈ 0 measurable overhead.** Achieved because every
mechanism is passive:

| Mechanism | Cost | Why |
|---|---|---|
| Crash detection | ~0 | OS process-exit / Job Object event. No polling. |
| Liveness | ~0 | Same OS event. |
| Responsiveness ping | negligible | Few bytes on the launcher pipe every 1–5 s; piggybacks the existing 20 s `mem_heartbeat`. |
| Rehydration source | **0 added writes** | Reads `objects.db`, which the reducer *already writes* for durability. |
| Crash/restart events | ~0 | Emitted only on a real crash. Crashes are rare. |
| Supervisor process | 0 | The launcher already exists and already supervises. |

Real work happens **only during an actual crash** — precisely when a few ms is
irrelevant.

### Forbidden (the performance traps)
- ❌ A parallel snapshot/checkpoint system for recovery. Rehydrate from
  existing persistence or not at all.
- ❌ Synchronous health checks on any request/hot path.
- ❌ High-frequency polling. Prefer OS events; cap pings at 1–5 s.
- ❌ Recovery logging routed through a pipe/log that is itself being recovered
  (the "don't measure the meter" feedback loop).

If the design ever needs a new write path *for recovery*, it has taken a wrong
turn — recovery free-rides on persistence, it does not add to it.

---

## 10. Failure modes & defenses — first-class

Recovery systems are notorious for becoming the most fragile part of the
system. Each known foot-gun and its mandated defense:

**A. Crashloop** — restart → re-crash → restart, forever.
→ Crash budget (N restarts per rolling window) + exponential backoff + a
**terminal state**. Never an infinite loop. (This is the project's loop-limit
discipline applied to processes.)

**B. Recovering into the poison** — rehydrate faithfully restores the state
that caused the crash.
→ §7 classification. Deterministic crashes restart from last-known-good /
safe mode, never blindly from crash-time state.

**C. False-positive kills** — a slow-but-healthy service flagged "hung" and
killed mid-work.
→ Conservative health (§8): generous timeouts, multiple missed pings, never
kill on one signal.

**D. Split-brain** — supervisor restarts a service that was actually alive ⇒
two writers to `objects.db`.
→ Single-writer discipline (the reducer stack already enforces this per layer)
+ Job Object + **generation/fencing tokens**: a stale instance's writes are
rejected by the persist layer.

**E. Cascading restarts** — srv dies → host can't reach srv → host deemed
unhealthy → host restarted → …
→ Services **tolerate a dependency being briefly absent** (reconnect, do not
die). Ordered, isolated restart: restart srv, let host *reconnect*.

**F. Masking real bugs** — silent auto-recovery hides a bug crashing 50×/day.
→ §12. Loud internally. Recovery *rate* is a P1 health metric, never a success
metric.

**G. Supervisor is now the most fragile thing** — complex, rarely exercised,
touches everything.
→ Prime directive (§3): the supervisor is the simplest, most-tested code in
the system. Recovery paths are fault-injection-tested in CI (§14) or they rot.

**H. Instrumentation feedback loop** — see §9 forbidden list.

---

## 11. Recovery flow (end to end)

```
service runs
  │
  ├─ liveness/responsiveness OK ──────────────────────────► (no-op, ~0 cost)
  │
  ├─ crash / hang detected
  │     │
  │     ├─ classify (§7)
  │     │     ├─ transient + within budget ─► restart ─► rehydrate ─► emit onUp
  │     │     ├─ deterministic ─────────────► degraded-mode restart
  │     │     └─ budget exhausted ──────────► safe mode ─► recovery UI
  │     │
  │     └─ every branch: count it, log it, surface it (§12)
  │
  └─ host-specific: GPU process crash ─► Chromium auto-restarts (works today);
        after N ─► launcher relaunches host — full config first,
        --disable-gpu only if the full relaunch also fails (§7 retry ladder)
```

For the host crash, the user-visible end state is **flicker-and-restore**: the
launcher detects the host's abnormal exit, suppresses the OS modal
(`SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS)` in host +
launcher), relaunches the host (full config first; `--disable-gpu` only if the
full relaunch reproduces the crash — §7 retry ladder), and the host reloads the
exact workspace/tabs/panes from `objects.db`.

---

## 12. Observability — loud internally

"Invisible to the user" must never mean "invisible to us."

- Every detection, classification, restart, and escalation is logged with the
  crash signature.
- A per-service **recovery counter** and **recovery rate** metric.
- The diagnostics panel surfaces recent recoveries (sibling of the reducer
  dispatch ring).
- A recovery rate above threshold is a **P1 alarm** — an auto-recovered crash
  is still a bug to fix, not a success.

---

## 13. Relationship to existing systems

The framework **composes with**, does not replace:

- **Reducer stack** — supplies the authoritative state `rehydrate()` replays.
  The framework is its dual (§5).
- **Saga coordinator** — already does crash-resume from `sagas.db`. The
  framework **generalizes** that pattern from "resume sagas" to "resume whole
  services." The coordinator becomes one supervised service among many.
- **WRR (Window Reality Reconciliation)** — already a reconciliation loop
  (model vs Win32 reality). Recovery reconciliation is its sibling; share the
  reconciliation primitives where possible.
- **Job Object / single-instance pipe** — already provide child-process
  containment and re-attach; the framework builds the restart policy on top.

---

## 14. Testing strategy — fault injection

Recovery code that is not continuously exercised rots, and untested recovery is
worse than none (§10-G). Therefore:

- A **fault-injection harness** that kills real processes (host, srv, GPU) and
  asserts: recovery completes, state is intact, the crash was counted, the user
  surface shows at most a flicker.
- These run in CI as a first-class suite, not a manual afterthought.
- Property-style: kill at random points (mid-write, mid-saga, mid-rehydrate);
  persistence atomicity (SQLite transactions) must hold.
- Crash classification tested with synthetic repeated signatures.

---

## 15. Phased rollout

Going app-wide in one step is itself a foot-gun. Prove the discipline on one
service first.

- **Phase 0 — Modal suppression.** `SetErrorMode` in host + launcher. Smallest
  change; immediately removes the scariest symptom. Ships independently.
- **Phase 1 — Host supervision (the case-study path).** Launcher detects host
  abnormal exit → restart → rehydrate from `objects.db`, re-attaching to the
  still-running srv (Tier 1, §6). Retry ladder per §7 — `--disable-gpu` only
  when a full relaunch reproduces a GPU-class crash. Crash budget +
  classification + recovery metric. Full
  fault-injection tests. **This is the proof of the whole pattern.**
- **Phase 2 — srv supervision.** A bespoke `SrvManager` (Tier 2, §6); host
  tolerates srv reconnect.
- **Phase 3 — Extract shared primitives.** Pull the primitives that ≥2 managers
  actually use (process-handle wrapper, crash-budget helper, classifier,
  recovery metric) into `agentmux-common` — bottom-up, never a uniform contract
  (§4). Fold the saga coordinator and Chromium-child handling in where it pays.
- **Phase 4 — Root hardening.** Decide (on evidence) whether the launcher needs
  an OS-level respawn.

Each phase ships only when the previous one is boringly reliable.

---

## 16. Open decisions

1. **Root supervisor** — leave the launcher as a bulletproof root, or add a
   minimal OS-level respawn? Default: leave it; revisit on evidence (§6, §15
   Phase 4).
2. **Crash-budget numbers** — N restarts per window, backoff curve, window
   length. Needs real crash-rate data.
3. **Last-known-good state** — do we keep periodic known-good markers in
   `objects.db`, or is "crash-time state minus the last command" enough? (Must
   not violate §9 — no new write path; likely a marker, not a copy.)
4. **`--disable-gpu-process-crash-limit`** — pass it always (Chromium keeps
   auto-restarting the GPU process instead of `LOG(FATAL)`), or only after the
   first GPU crash?
5. **Hang detection budget** — ping interval and missed-ping threshold;
   needs measurement so legit long operations are never misclassified.
6. **Frontend (Layer 4)** — is a frozen renderer in scope here, or handled by
   Chromium reload + slice rehydrate alone?

---

## 17. References

- `docs/analysis/CRASH_GPU_PROCESS_FATAL_2026_05_20.md` — case study #1.
- `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md` — reducer stack.
- `docs/specs/SPEC_PHASE_E_SAGAS_2026-04-30.md` — saga crash-resume precedent.
- Discussion #707 — reducer-stack tracking thread (model for this spec's
  tracking thread).
