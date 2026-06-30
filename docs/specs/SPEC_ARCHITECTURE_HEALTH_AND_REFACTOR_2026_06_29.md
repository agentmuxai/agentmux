# Architecture Health Assessment & Refactor Proposal

**Date:** 2026-06-29
**Status:** Assessment + recommendation (decision doc)
**Scope:** Process/memory model, lifecycle ownership, state durability — the substrate under the recurring OOM and teardown failures.
**Method:** Four parallel deep audits (process/memory, lifecycle/sagas, state/persistence, historical trend), cross-referenced against the existing memory-pressure corpus.

---

## 0. TL;DR

The recurring OOM crashes and the lifecycle/teardown churn are **not two problems — they're one root cause with two faces.** That root is a single architectural decision:

> **The CEF host is simultaneously (a) the component most likely to die — Chromium aborts the process on any failed allocation (`0xE0000008`) — and (b) the authoritative in-memory owner of session, UI, and lifecycle state.**

Because the fragile thing is also the irreplaceable thing, **every death is catastrophic and every recovery is a hand-built special case.** That is why we keep shipping band-aids (gated recovery, memory supervisor, crash budgets, pause pages, graceful-exit) — each one papers over one consequence of the same inversion.

**Verdict:** Two subsystems are **NEEDS-REFACTOR** (process/memory admission model; lifecycle quit-authority). One is **STRAINED / pay-down-debt** (persistence). This warrants a **targeted structural refactor, not a rewrite** — and notably, the two highest-value fixes are *already written or specced by the team and sitting unmerged.*

---

## 1. The evidence is no longer anecdotal — it's a measured trend

From the historical audit (PR-number proxy, since the repo is squash-imported with useless timestamps):

- **~45–55 of every 100 PRs** touch memory/lifecycle/crash — and that rate is **flat-to-rising over ~1,000 PRs**, never tapering. A stabilizing system shows decline; this doesn't.
- **~1 in 5 commits is a `follow-up` (147) or `revert` (40).**
- **Repeated re-fixes of the same bug class:** renderer-OOM recovery fixed **6+ times** (#1120→#1124→#1121→#1229→#1230→#1493→#1799); replaceChild crash **4 times** (the 4th literally named `FULL_ANALYSIS_AND_FIX`); cross-channel continuity **15 commits** with a retro titled *"fixed then broke again"*; zoom **8 docs** with per-platform re-breakage.
- **29+ dated memory/lifecycle/crash docs span 2026-03-26 → today, with no gap**, densest in June. **The two newest specs in the entire repo are both OOM specs** written today.

This is the signature of mitigation on an unstable base, not polish on a stable one.

---

## 2. Per-subsystem findings

### 2.1 Process & memory model — **NEEDS-REFACTOR**
- **No proactive admission control.** Agent turns spawn `claude.exe` **unconditionally** (`agents/runner.rs:142`, `drone/executor/blocks/agent.rs:96`). The only process that reads system commit (`mem_supervisor.rs:110`) does so **after** the host has already died, to classify the corpse. The model is **spawn → overcommit → Chromium aborts → relaunch.** Reactive by construction.
- **The host is a single point of total UI-state loss.** All pane/window/drag/pool state lives in volatile host memory (`agentmux-cef/src/state.rs`, `reducer/mod.rs`). The 2026-06-26 incident: host working set was ~120 MB while the box was 99.9% committed — **the host was the victim, not the cause**, yet it's the thing that dies and loses everything.
- **Agents are tracked but never capped.** Job Objects guarantee reap (a real strength) but nothing kills an agent on memory pressure; an overnight `claude.exe` is what filled the pagefile.
- **The team specced the fix and hasn't built it:** a "commit-aware turn scheduler — gate spawn on commit headroom" is listed **P0** in `SPEC_WIN10_PAGEFILE_OOM_CRASH` and is **not shipped.**
- *Honest admission in-tree* (`mem_supervisor.rs:4-16`): the relaunch ladder is "memory-blind: on a system OOM it relaunches straight back into the same commit-starved condition, burns the budget in seconds, and gives up — a silent vanish."

### 2.2 Lifecycle ownership & sagas — **NEEDS-REFACTOR**
- **Spawn/reap is sound** (launcher + Windows Job Object `KILL_ON_JOB_CLOSE`, never regressed). The rot is in the **quit decision**, which is split across **3–4 competing sites**: `client::on_before_close` (edge-triggered), the WRR `maybe_quit_on_last_user_window` path, `orphan_reconcile.rs`, and the intended replacement `reducer/quit.rs::reconcile_quit` — **which is written, tested, and `#[allow(dead_code)]` / "NOT YET WIRED."**
- **The central regression is understood and unreverted:** close was changed from authoritative `host.try_close_browser()` to an HWND-guess `PostMessage(WM_CLOSE)` that returns NULL on CEF Views → frame hides without closing → `on_before_close` never fires → Job Object never drops → **whole tree orphans** (`retro-lifecycle-teardown-churn-2026-06-22`).
- **Recovery philosophy is incoherent — both "let-it-crash" and "never-crash" in the same file.** The crash path asserts srv state is durable ("Resume re-projects everything"); the graceful path asserts it is *not* durable and must synchronously flush shells before close. They don't compose — which is *why* `orphan_reconcile.rs` had to be invented.
- **The saga layer is over-engineered for the job:** ~**3,986 lines** of SQLite-durable distributed-transaction machinery (journal, per-step log, restart walker) for **only 2 saga types**, and it **doesn't even compensate** — on restart it marks sagas `failed_compensation` for an operator to read. ~38 phase-markers / 4 full re-architectures of the quit path in ~9 months.

### 2.3 State & persistence — **STRAINED (pay down debt, no rethink)**
- Foundation is actually decent: four well-bounded SQLite stores, schema-version safety locks, pre-migration snapshots, and — notably — **no duplicated domain state in the frontend** (caches are in-memory only).
- **Two concentrated strains:**
  1. **Stalled agents dual-write migration.** `db_agent_definitions/_instances` (old, still the read source) are mirrored into `db_agents` on every mutation across **11+ call sites**; `dual_write.rs` (679 lines) + `agents_consolidate.rs` (942 lines) + a **per-startup `repair_def_gaps()`** exist solely to bridge a cutover whose Phase 3b/3c **are written but unmerged.** ~1,600 lines of transitional scaffolding waiting on two PRs.
  2. **Layout split-brain.** One `db_layout` record written through **two paths** (reducer for focus/magnify, "wcore-direct" for the tree) and mirrored into 3 in-memory copies, kept consistent by hand-ordered code rather than an invariant.
- **Durability gap on abrupt kill:** WAL checkpoint runs only on a 30-min timer (no checkpoint on shutdown) → WAL bloat (not data loss); in-flight PTY buffer and queued messages are explicitly lost on kill.

---

## 3. The unifying diagnosis

Map the four audits onto one picture:

```
        Chromium aborts on OOM (0xE0000008)         ← substrate we can't change
                     │
   the HOST is the process that dies ───────────────┐
                     │                               │
   …but the HOST also OWNS, in volatile memory:      │
     • session/pane/window/pool/drag state           │  ⇒ every host death is
     • the lifecycle "should we quit?" decision        catastrophic + bespoke
     • the contract that shells/state are flushed     │
                     │                               │
   ⇒ recovery must be hand-built per failure ────────┘
   ⇒ saga layer exists to compensate for lost work
   ⇒ no admission control, because "just relaunch the owner"
```

Every band-aid we've shipped is a local fix to one arrow in that diagram. The arrows all originate from one node: **the fragile process is the stateful authority.**

---

## 4. Refactor thesis (the structural move)

**Invert the relationship: make the durable backend the authority, and make the host a disposable projection.** Three pillars, each of which *collapses* a class of current band-aids rather than adding another.

### Pillar 1 — Host becomes stateless & reprojectable (kills the "catastrophe" class)
Persist the full session/UI/window topology to srv (which already survives host death). The host holds only a projection it can rebuild at any instant. Then **host OOM stops being a catastrophe** — it's a reproject, the same as a renderer crash.
- *Collapses:* the graceful-shutdown-vs-crash incoherence (always crash-and-reproject), the synchronous shell-flush contract, and most of the bespoke recovery pages.
- *Builds on:* the existing "srv is durable, Resume re-projects" assumption — just make it actually true for **all** host state, not most.

### Pillar 2 — Single level-triggered lifecycle authority (kills the "teardown churn" class)
Wire `reconcile_quit` (already written/tested) as the **sole** writer of quit state, re-evaluated after every window/pool/creation transition. Demote `on_before_close`, the WRR path, and `orphan_reconcile` to pure executors.
- *Collapses:* the 3–4-way quit split-brain, the orphan-reconciliation module, the pool-refill-vs-last-window race that's resurfaced under new symptoms 5+ times.
- *And then:* with the host stateless (Pillar 1) there is **nothing to compensate**, so the ~4,000-line saga durability layer can collapse to an in-memory registry.

### Pillar 3 — Proactive admission control + per-agent caps (kills the "OOM" class at the source)
One budget owner (srv) gates `claude.exe`/turn spawning on **available commit before launching**, queues when headroom is short, and enforces a per-agent working-set cap with kill-and-reproject. Track free disk on the pagefile volume (the variable today's gauge misses).
- *Collapses:* the memory supervisor's relaunch ladder, crash budgets, pause-page budgets, magic floors (`RESUME_FLOOR_MB`/`PAINT_FLOOR_MB`) — overcommit simply stops happening.
- *Builds on:* the P0 "commit-aware turn scheduler" the team already specced.

### Pay-down (not a pillar, but bundle it): land agents Phase 3b/3c, unify the layout write path.

---

## 5. Why this is a refactor, not a rewrite
- Pillars 1–3 reuse what's sound: Job Object reap, the four SQLite stores, the reducer pattern, the srv websocket projection.
- **The two highest-leverage pieces already exist:** `reconcile_quit` (written, unwired) and the commit-aware scheduler (specced, P0). This is finishing started work, not greenfield.
- The saga collapse is a *deletion*, which is the safest kind of change.

## 6. Recommendation
1. **Stop adding OOM/lifecycle band-aids** (including pausing the just-written `SPEC_GRACEFUL_OOM_EXIT` beyond its cheap P0 dialog) until Pillars 1–3 are sequenced. The graceful-exit work becomes *much* smaller once host death is a reproject.
2. **Sequence:** Pillar 2 first (it's mostly wiring already-written code and stops active orphan/teardown bleeding) → Pillar 3 (stops the OOM at the source) → Pillar 1 (the deepest change; makes the rest collapse) → saga collapse + persistence pay-down.
3. **Add the missing E2E test** ("close last window ⇒ tree exits", "host OOM ⇒ session reprojects") — the teardown retro notes every regression shipped silently for lack of one.

## 7. Risks / honest caveats
- **Pillar 1 is genuinely deep** — serializing all host UI state to srv and reprojecting cleanly is the hard part; do it last, behind the cheaper wins.
- The persistence audit rated state **STRAINED, not refactor** — don't over-rotate into rewriting the data layer; it mostly needs the stalled migration *finished*.
- CEF coupling stays deep regardless; this proposal makes it *tolerable* (host disposable) rather than removing it.
- Estimates are not in this doc on purpose — this is a direction decision; each pillar needs its own sized spec.

## 8. Sources / supporting docs
- `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`, `docs/specs/SPEC_GRACEFUL_OOM_EXIT_2026_06_29.md`
- `docs/incident/INCIDENT_2026_06_26_APP_CLOSED.md`, `docs/specs/SPEC_MEMORY_ANALYSIS_2026_06_26.md`
- `docs/retro/retro-lifecycle-teardown-churn-2026-06-22.md`, `docs/specs/SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md`
- `docs/specs/SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md`, `docs/specs/SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`
- `reducer/quit.rs` (the unwired `reconcile_quit`), `agentmux-launcher/src/saga/` (the collapsible layer)
- Phase 3b/3c agents migration (unmerged), `agents_consolidate.rs` / `dual_write.rs` (transitional scaffolding)
